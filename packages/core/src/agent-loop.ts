import { ApiClient } from "./client.ts";
import { parseSseLine } from "./sse.ts";
import type {
  ChatMessage,
  StreamChunk,
  StreamChoice,
  ToolCall,
} from "./types.ts";

// ── Stream result ────────────────────────────────────────────────────────────

export interface StreamResult {
  content: string;
  reasoningContent: string;
  toolCalls: ToolCall[];
  hasToolCalls: boolean;
  finishReason: string;
}

// ── Tool result ─────────────────────────────────────────────────────────────

export interface ToolResult {
  success: boolean;
  outputType?: string;
  content?: string;
  error?: string;
}

// ── Agent loop callbacks ─────────────────────────────────────────────────────

export interface AgentCallbacks {
  /** Called for each content delta during streaming */
  onContentDelta(content: string): void;
  /** Called for each reasoning delta */
  onReasoningDelta(content: string): void;
  /** Called when a tool call is about to execute */
  onToolStart(name: string, args: Record<string, unknown>): void;
  /** Called after tool execution */
  onToolResult(name: string, result: ToolResult): void;
  /** Called when permission is needed */
  onPermissionRequired(
    reason: string,
    sessionRule: string
  ): Promise<"allow" | "always" | "deny">;
  /** Called when ask_user_question is invoked */
  onAskUserQuestion(
    question: string,
    options: { label: string; description: string }[],
    multiSelect: boolean
  ): Promise<string[]>;
}

// ── Stream processor (TypeScript port of Rust StreamProcessor) ──────────────

class StreamProcessor {
  private buffer = "";
  fullContent = "";
  reasoningContent = "";
  private toolCallsAccum: Record<string, unknown>[] = [];
  hasToolCalls = false;
  private finishReason = "";

  feedBytes(bytes: Uint8Array): StreamEvent[] {
    this.buffer += new TextDecoder().decode(bytes);
    return this.drainBuffer();
  }

  private drainBuffer(): StreamEvent[] {
    const events: StreamEvent[] = [];
    let idx: number;
    while ((idx = this.buffer.indexOf("\n")) !== -1) {
      const line = this.buffer.slice(0, idx).trim();
      this.buffer = this.buffer.slice(idx + 1);

      const event = this.processLine(line);
      if (event) events.push(event);
    }
    return events;
  }

  private processLine(line: string): StreamEvent | null {
    const chunk = parseSseLine(line);
    if (!chunk) return null;

    const choice = chunk.choices[0];
    if (!choice) return null;

    // Finish reason
    if (choice.finish_reason) {
      this.finishReason = choice.finish_reason;
      return { type: "done", finishReason: choice.finish_reason };
    }

    // Content delta
    if (choice.delta.content) {
      this.fullContent += choice.delta.content;
      return { type: "content", content: choice.delta.content };
    }

    // Reasoning delta
    if (choice.delta.reasoning_content) {
      this.reasoningContent += choice.delta.reasoning_content;
      return { type: "reasoning", content: choice.delta.reasoning_content };
    }

    // Tool call deltas
    if (choice.delta.tool_calls) {
      this.hasToolCalls = true;
      for (const tc of choice.delta.tool_calls) {
        const idx = tc.index;
        while (this.toolCallsAccum.length <= idx) {
          this.toolCallsAccum.push({
            id: null,
            type: "function",
            function: { name: "", arguments: "" },
          });
        }
        const entry = this.toolCallsAccum[idx] as Record<string, unknown>;
        if (tc.id) entry.id = tc.id;
        if (tc.function) {
          const func = entry.function as Record<string, string>;
          if (tc.function.name) func.name = tc.function.name;
          if (tc.function.arguments)
            func.arguments = (func.arguments || "") + tc.function.arguments;
        }

        return {
          type: "tool_call_delta",
          index: idx,
          id: tc.id ?? undefined,
          name: tc.function?.name ?? undefined,
          arguments: tc.function?.arguments ?? undefined,
        };
      }
    }

    return null;
  }

  finish(): StreamResult {
    const toolCalls: ToolCall[] = this.toolCallsAccum
      .filter((c) => c.id)
      .map((c) => {
        const func = (c as Record<string, unknown>).function as Record<
          string,
          string
        >;
        return {
          id: c.id as string,
          type: "function" as const,
          function: {
            name: func.name,
            arguments: func.arguments,
          },
        };
      });

    return {
      content: this.fullContent,
      reasoningContent: this.reasoningContent,
      toolCalls,
      hasToolCalls: this.hasToolCalls,
      finishReason: this.finishReason,
    };
  }
}

// ── Stream event ─────────────────────────────────────────────────────────────

type StreamEvent =
  | { type: "content"; content: string }
  | { type: "reasoning"; content: string }
  | {
      type: "tool_call_delta";
      index: number;
      id?: string;
      name?: string;
      arguments?: string;
    }
  | { type: "done"; finishReason: string };

// ── Agent loop ───────────────────────────────────────────────────────────────

export interface AgentLoopOptions {
  client: ApiClient;
  callbacks: AgentCallbacks;
  sessionId?: string;
  maxTokens?: number;
}

export class AgentLoop {
  private client: ApiClient;
  private callbacks: AgentCallbacks;
  private sessionId: string;
  private maxTokens?: number;
  conversationHistory: ChatMessage[] = [];

  constructor(options: AgentLoopOptions) {
    this.client = options.client;
    this.callbacks = options.callbacks;
    this.sessionId = options.sessionId ?? "default";
    this.maxTokens = options.maxTokens;
  }

  /** Process a single user input. Handles the full agent loop (SSE + tools). */
  async processInput(input: string): Promise<void> {
    this.conversationHistory.push({ role: "user", content: input });

    const maxRounds = 10;
    for (let round = 0; round < maxRounds; round++) {
      const messages = [...this.conversationHistory];

      const response = await this.client.chatStream({
        messages,
        max_tokens: this.maxTokens,
      });

      // Stream SSE
      const result = await this.streamResponse(response);

      if (result.hasToolCalls && result.toolCalls.length > 0) {
        // Build assistant message
        const assistantMsg: ChatMessage = {
          role: "assistant",
          content: result.content || null,
          reasoning_content: result.reasoningContent || null,
          tool_calls: result.toolCalls,
          tool_call_id: null,
        };
        this.conversationHistory.push(assistantMsg);

        // Execute each tool
        for (const tc of result.toolCalls) {
          let args: Record<string, unknown>;
          try {
            args = JSON.parse(tc.function.arguments);
          } catch {
            args = {};
          }

          if (tc.function.name === "ask_user_question") {
            const toolResult = await this.handleAskUserQuestion(args);
            this.conversationHistory.push({
              role: "tool",
              content: toolResult,
              tool_call_id: tc.id,
            });
            continue;
          }

          this.callbacks.onToolStart(tc.function.name, args);

          const execResult = await this.executeToolWithPermission(
            tc.function.name,
            args
          );

          this.callbacks.onToolResult(tc.function.name, execResult);

          const toolResultStr = JSON.stringify(execResult);
          this.conversationHistory.push({
            role: "tool",
            content: toolResultStr,
            tool_call_id: tc.id,
          });
        }

        // Continue loop — model gets tool results
        continue;
      }

      // Normal response
      if (result.content) {
        this.conversationHistory.push({
          role: "assistant",
          content: result.content,
          reasoning_content: result.reasoningContent || null,
        });
      }
      break;
    }
  }

  private async streamResponse(response: Response): Promise<StreamResult> {
    const processor = new StreamProcessor();
    const reader = response.body?.getReader();
    if (!reader) throw new Error("Response body is not readable");

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        for (const event of processor.feedBytes(value)) {
          switch (event.type) {
            case "content":
              this.callbacks.onContentDelta(event.content);
              break;
            case "reasoning":
              this.callbacks.onReasoningDelta(event.content);
              break;
            case "tool_call_delta":
              // silently accumulate
              break;
            case "done":
              // stream done
              break;
          }
        }
      }
    } finally {
      reader.releaseLock();
    }

    return processor.finish();
  }

  private async executeToolWithPermission(
    name: string,
    args: Record<string, unknown>
  ): Promise<ToolResult> {
    // First attempt — execute directly
    const result = await this.client.executeTool({
      tool_name: name,
      arguments: args,
      session_id: this.sessionId,
    });

    // Check if permission is required
    if (result.permission_required) {
      const choice = await this.callbacks.onPermissionRequired(
        result.permission_required.reason,
        result.permission_required.session_rule
      );

      if (choice === "deny") {
        return {
          success: false,
          error: `PERMISSION DENIED: The user rejected '${name}'.`,
        };
      }

      // Approve the rule so the daemon allows the retry
      await this.client.approveTool(
        result.permission_required.session_rule
      );

      // Retry after approval
      const retryResult = await this.client.executeTool({
        tool_name: name,
        arguments: args,
        session_id: this.sessionId,
      });

      return {
        success: retryResult.success,
        outputType: retryResult.output_type ?? undefined,
        content: retryResult.content ?? undefined,
      };
    }

    return {
      success: result.success,
      outputType: result.output_type ?? undefined,
      content: result.content ?? undefined,
    };
  }

  private async handleAskUserQuestion(
    args: Record<string, unknown>
  ): Promise<string> {
    const question = (args.question as string) ?? "Select an option:";
    const rawOptions = args.options as
      | { label: string; description: string }[]
      | undefined;
    const multiSelect = (args.multiSelect as boolean) ?? false;

    const options = rawOptions ?? [];
    const answers = await this.callbacks.onAskUserQuestion(
      question,
      options,
      multiSelect
    );

    return JSON.stringify({
      success: true,
      answers: answers.map((label) => ({ label, value: label, custom: false })),
    });
  }

  /** Reset conversation history */
  reset(): void {
    this.conversationHistory = [];
  }
}
