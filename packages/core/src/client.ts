import type {
  ChatStreamRequest,
  ConfigResponse,
  ExecuteToolRequest,
  ExecuteToolResponse,
  HealthResponse,
  ToolInfo,
} from "./types.ts";

export interface ClientOptions {
  baseUrl: string;
}

/**
 * HTTP client for the Rust daemon API.
 */
export class ApiClient {
  private baseUrl: string;

  constructor(options: ClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/$/, "");
  }

  // ── Health ───────────────────────────────────────────────────────────────

  async health(): Promise<HealthResponse> {
    const res = await fetch(`${this.baseUrl}/api/v1/health`);
    return res.json();
  }

  // ── Config ───────────────────────────────────────────────────────────────

  async getConfig(): Promise<ConfigResponse> {
    const res = await fetch(`${this.baseUrl}/api/v1/config`);
    return res.json();
  }

  // ── Chat / Stream ────────────────────────────────────────────────────────

  /**
   * Send a streaming chat request. Returns the raw Response for SSE reading.
   */
  async chatStream(request: ChatStreamRequest): Promise<Response> {
    const res = await fetch(`${this.baseUrl}/api/v1/chat/stream`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request),
    });

    if (!res.ok) {
      const body = await res.text();
      throw new Error(`API error (${res.status}): ${body}`);
    }

    return res;
  }

  // ── Tools ────────────────────────────────────────────────────────────────

  async listTools(): Promise<{ tools: ToolInfo[] }> {
    const res = await fetch(`${this.baseUrl}/api/v1/tools`);
    return res.json();
  }

  async executeTool(request: ExecuteToolRequest): Promise<ExecuteToolResponse> {
    const res = await fetch(`${this.baseUrl}/api/v1/tools/execute`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request),
    });

    if (!res.ok) {
      throw new Error(`Tool execution failed (${res.status})`);
    }

    return res.json();
  }

  async approveTool(sessionRule: string): Promise<void> {
    await fetch(`${this.baseUrl}/api/v1/tools/approve`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_rule: sessionRule }),
    });
  }
}
