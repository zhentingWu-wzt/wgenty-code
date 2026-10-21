import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight, FileMinus, FileText, Folder, Loader2 } from "lucide-react";
import { toast } from "sonner";
import type { DaemonClient } from "../../api/client";
import type { FsEntries, GitChangeKind } from "../../api/types";
import { cn } from "../../lib/utils";
import { useUiStore } from "../../state/uiStore";

/**
 * Workspace file tree (design doc §1.4) — read-only browsing of one workspace
 * root (main checkout or a linked worktree task), mounted by the right rail's
 * FilesPanel with the active session's workspace root.
 *
 * Lazy loading: the root listing is fetched on mount, each sub-directory
 * fetches on its own first expand, and listings are cached by directory path —
 * collapsing never refetches.
 *
 * Git change coloring: changed files (added/modified/deleted, untracked
 * included) are colored by name; directories aggregate the strongest change
 * beneath them; deleted files keep rendering at their old parent even though
 * they no longer exist on disk.
 */

/** Daemon-side listing cap (src/daemon/workspace_files.rs). */
const LIST_LIMIT = 2000;

/** Generated dirs that are usually build output / deps: greyed out and stay
 *  collapsed by default (expanding is still allowed). */
const MUTED_DIR_NAMES = new Set(["target", "node_modules", "dist"]);

export function isMutedDir(name: string): boolean {
  return MUTED_DIR_NAMES.has(name);
}

/** Tailwind text color per change kind — the single source of tree coloring. */
export const STATUS_COLOR: Record<GitChangeKind, string> = {
  added: "text-success",
  modified: "text-warning",
  deleted: "text-danger",
};

/** Extensions treated as text for the tab's first-guess `kind`. The preview
 *  panel re-checks against the actual fetchFile response, so this only picks
 *  the initial icon/fallback — unknown or non-text extensions guess "binary". */
const TEXT_EXTENSIONS = new Set([
  // systems languages
  "rs",
  "go",
  "zig",
  "py",
  "rb",
  "java",
  "kt",
  "swift",
  "scala",
  "dart",
  "c",
  "h",
  "cpp",
  "hpp",
  "cc",
  "hh",
  "cs",
  "m",
  "mm",
  "php",
  "lua",
  "pl",
  "r",
  // shell
  "sh",
  "bash",
  "zsh",
  "fish",
  "ps1",
  "bat",
  "cmd",
  // web / frontend
  "ts",
  "tsx",
  "js",
  "jsx",
  "mjs",
  "cjs",
  "vue",
  "svelte",
  "html",
  "htm",
  "css",
  "scss",
  "sass",
  "less",
  // data & config
  "json",
  "jsonc",
  "json5",
  "toml",
  "yaml",
  "yml",
  "xml",
  "ini",
  "cfg",
  "conf",
  "env",
  "properties",
  "lock",
  "sql",
  "csv",
  "tsv",
  // docs & text
  "md",
  "mdx",
  "txt",
  "rst",
  "adoc",
  "log",
  "diff",
  "patch",
]);

/** Common extensionless files that are text (matched lowercased). */
const TEXT_FILENAMES = new Set([
  "makefile",
  "dockerfile",
  "license",
  "licence",
  "readme",
  "changelog",
  "notice",
  "authors",
  "contributors",
  "codeowners",
  ".gitignore",
  ".gitattributes",
  ".gitmodules",
  ".dockerignore",
  ".editorconfig",
  ".env",
  ".npmrc",
  ".nvmrc",
  ".tool-versions",
]);

/** First-guess preview kind by file name; the panel re-checks on fetch. */
export function guessFileKind(name: string): "text" | "binary" {
  const lower = name.toLowerCase();
  const dot = lower.lastIndexOf(".");
  if (dot > 0) return TEXT_EXTENSIONS.has(lower.slice(dot + 1)) ? "text" : "binary";
  return TEXT_FILENAMES.has(lower) ? "text" : "binary";
}

/** `dir + "/" + name` without doubling the slash at filesystem roots. */
export function joinPath(dir: string, name: string): string {
  return dir.endsWith("/") ? `${dir}${name}` : `${dir}/${name}`;
}

/** Parent directory of a `/`-joined rel path ("" at the root). */
export function parentOfRel(rel: string): string {
  const idx = rel.lastIndexOf("/");
  return idx === -1 ? "" : rel.slice(0, idx);
}

/** Path of `abs` relative to workspace `root` ("" at the root itself).
 *  Falls back to `abs` unchanged if it is somehow not under the root. */
export function relativeTo(root: string, abs: string): string {
  if (root === "/") return abs.startsWith("/") ? abs.slice(1) : abs;
  const trimmed = root.endsWith("/") ? root.slice(0, -1) : root;
  if (abs === trimmed) return "";
  if (abs.startsWith(`${trimmed}/`)) return abs.slice(trimmed.length + 1);
  return abs;
}

/** Aggregate change kinds up the directory chain, keeping the strongest:
 *  deleted > modified > added (see `KIND_RANK`). */
export function aggregateDirStatus(
  statusByRel: Record<string, GitChangeKind>,
): Map<string, GitChangeKind> {
  const rank: Record<GitChangeKind, number> = { added: 1, modified: 2, deleted: 3 };
  const out = new Map<string, GitChangeKind>();
  for (const [rel, kind] of Object.entries(statusByRel)) {
    let dir = parentOfRel(rel);
    while (dir) {
      const prev = out.get(dir);
      if (!prev || rank[kind] > rank[prev]) out.set(dir, kind);
      dir = parentOfRel(dir);
    }
  }
  return out;
}

export function FileTree({
  workspaceRoot,
  client,
}: {
  /** Task workspace root: worktree path or main-checkout project path. */
  workspaceRoot: string;
  client: DaemonClient;
}) {
  /** Listings cached by (canonical) directory path. */
  const [entriesByDir, setEntriesByDir] = useState<Record<string, FsEntries>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [loadingDirs, setLoadingDirs] = useState<Set<string>>(new Set());
  const [failedDirs, setFailedDirs] = useState<Set<string>>(new Set());
  /** Changed files by root-relative path (""-rooted, forward slashes). */
  const [statusByRel, setStatusByRel] = useState<Record<string, GitChangeKind>>({});
  /** Canonical root as reported by the daemon (`FsEntries.current` may differ
   *  from the requested path, e.g. macOS /tmp → /private/tmp); relPaths and
   *  preview-tab roots are computed against it. */
  const [rootCanonical, setRootCanonical] = useState<string | null>(null);
  const rootForRel = rootCanonical ?? workspaceRoot;

  const load = useCallback(
    async (dir: string) => {
      setLoadingDirs((s) => new Set(s).add(dir));
      setFailedDirs((s) => {
        if (!s.has(dir)) return s;
        const next = new Set(s);
        next.delete(dir);
        return next;
      });
      try {
        const res = await client.listEntries(dir);
        setEntriesByDir((m) => ({ ...m, [dir]: res }));
        if (dir === workspaceRoot && res.current !== rootCanonical) {
          setRootCanonical(res.current);
        }
      } catch (e) {
        toast.error(`加载目录失败 ${dir}: ${e instanceof Error ? e.message : String(e)}`);
        setFailedDirs((s) => new Set(s).add(dir));
      } finally {
        setLoadingDirs((s) => {
          const next = new Set(s);
          next.delete(dir);
          return next;
        });
      }
    },
    // rootCanonical is intentionally not a dependency: it only transitions
    // once and re-running the root fetch for it would be wasteful.
    //
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [client, workspaceRoot],
  );

  // The panel mounts FileTree only when opened, so this mount effect IS the
  // "first open sends the first listEntries" gate.
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount
    void load(workspaceRoot);
  }, [load, workspaceRoot]);

  // Git change colors: one fetch per workspace root. Non-git roots return []
  // and errors degrade silently to "no colors" — coloring is cosmetic.
  const loadStatus = useCallback(async () => {
    try {
      const list = await client.gitStatus(rootForRel);
      setStatusByRel(Object.fromEntries(list.map((s) => [s.path, s.status])));
    } catch {
      setStatusByRel({});
    }
  }, [client, rootForRel]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount
    void loadStatus();
  }, [loadStatus]);

  /** dir rel-path → strongest change beneath it (for DirNode coloring). */
  const dirStatusByRel = useMemo(() => aggregateDirStatus(statusByRel), [statusByRel]);

  const toggleDir = (dir: string) => {
    const isOpen = expanded.has(dir);
    setExpanded((s) => {
      const next = new Set(s);
      if (isOpen) next.delete(dir);
      else next.add(dir);
      return next;
    });
    // First expand of an uncached directory triggers its lazy load.
    if (!isOpen && !entriesByDir[dir]) void load(dir);
  };

  const openFile = (dir: string, name: string) => {
    const absPath = joinPath(dir, name);
    useUiStore.getState().openPreviewTab({
      workspaceRoot: rootForRel,
      absPath,
      relPath: relativeTo(rootForRel, absPath),
      kind: guessFileKind(name),
    });
  };

  // ── Rendering ────────────────────────────────────────────────────────────

  const renderDirBody = (dir: string): ReactNode => {
    if (loadingDirs.has(dir)) {
      return (
        <div className="flex items-center gap-1 px-1 py-0.5 text-[13px] text-muted-foreground">
          <Loader2 size={12} className="animate-spin" />
          <span>加载中…</span>
        </div>
      );
    }
    if (failedDirs.has(dir)) {
      return (
        <button
          type="button"
          className="rounded-sm px-1 py-0.5 text-left text-[13px] text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={() => void load(dir)}
        >
          加载失败，点击重试
        </button>
      );
    }
    const listing = entriesByDir[dir];
    if (!listing) return null;
    // Children join off the daemon-reported canonical path so tab absPaths
    // stay aligned with the canonical workspace root.
    const parent = listing.current ?? dir;
    return (
      <>
        {listing.entries.map((e) => {
          const absPath = joinPath(parent, e.name);
          const rel = relativeTo(rootForRel, absPath);
          if (e.is_dir) {
            const isOpen = expanded.has(absPath);
            return (
              <DirNode
                key={e.name}
                dir={absPath}
                name={e.name}
                muted={isMutedDir(e.name)}
                status={dirStatusByRel.get(rel)}
                isOpen={isOpen}
                onToggle={() => toggleDir(absPath)}
                body={isOpen ? renderDirBody(absPath) : null}
              />
            );
          }
          return (
            <FileRow
              key={e.name}
              name={e.name}
              absPath={absPath}
              status={statusByRel[rel]}
              onOpen={() => openFile(parent, e.name)}
            />
          );
        })}
        {/* Deleted files keep their old parent visible even though they no
            longer appear in the on-disk listing. */}
        {deletedIn(dir).map((name) => (
          <DeletedFileRow key={`deleted:${name}`} name={name} />
        ))}
        {listing.truncated && (
          <div className="truncate px-1 py-0.5 text-[11px] text-muted-foreground">
            已截断，仅显示前 {LIST_LIMIT} 项
          </div>
        )}
      </>
    );
  };

  /** Deleted status entries whose parent is `dir` and that are absent from
   *  its (already-loaded) listing — rendered as strike-through rows. */
  const deletedIn = (dir: string): string[] => {
    const parentRel = relativeTo(rootForRel, entriesByDir[dir]?.current ?? dir);
    const listing = entriesByDir[dir];
    return Object.entries(statusByRel)
      .filter(([rel, kind]) => kind === "deleted" && parentOfRel(rel) === parentRel)
      .map(([rel]) => rel.slice(rel.lastIndexOf("/") + 1))
      .filter((name) => !listing?.entries.some((e) => e.name === name))
      .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
  };

  return (
    <div className="flex flex-col gap-0.5" data-testid="file-tree" data-root={workspaceRoot}>
      {renderDirBody(workspaceRoot)}
    </div>
  );
}

/** One directory row. Module scope (not inside FileTree) so the component
 *  identity is stable across re-renders — React keeps the DOM node, which
 *  in-flight state updates (git status arriving late) must not replace
 *  mid-click. */
function DirNode({
  dir,
  name,
  muted,
  status,
  isOpen,
  onToggle,
  body,
}: {
  dir: string;
  name: string;
  muted: boolean;
  status?: GitChangeKind;
  isOpen: boolean;
  onToggle: () => void;
  /** Pre-computed children (null while collapsed). */
  body: ReactNode;
}) {
  return (
    <div>
      <div className="group flex items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent">
        <button
          type="button"
          aria-expanded={isOpen}
          title={dir}
          onClick={onToggle}
          className="flex min-w-0 flex-1 items-center gap-1 text-left text-[13px]"
        >
          {isOpen ? (
            <ChevronDown size={12} className="shrink-0" />
          ) : (
            <ChevronRight size={12} className="shrink-0" />
          )}
          <Folder
            size={13}
            className={cn(
              "shrink-0",
              muted ? "text-muted-foreground" : status && STATUS_COLOR[status],
            )}
          />
          <span
            className={cn(
              "truncate",
              muted && "text-muted-foreground",
              status && STATUS_COLOR[status],
            )}
          >
            {name}
          </span>
        </button>
      </div>
      {isOpen && <div className="ml-4 flex flex-col gap-0.5">{body}</div>}
    </div>
  );
}

/** One file row. Module scope for the same identity reason as DirNode. */
function FileRow({
  name,
  absPath,
  status,
  onOpen,
}: {
  name: string;
  absPath: string;
  status?: GitChangeKind;
  onOpen: () => void;
}) {
  return (
    <div className="flex items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent">
      <button
        type="button"
        title={absPath}
        onClick={onOpen}
        className="flex min-w-0 flex-1 items-center gap-1 text-left text-[13px]"
      >
        {/* Chevron-width spacer keeps file names aligned with dir names. */}
        <span className="w-3 shrink-0" />
        <FileText size={13} className={cn("shrink-0", status && STATUS_COLOR[status])} />
        <span className={cn("truncate", status && STATUS_COLOR[status])}>{name}</span>
      </button>
    </div>
  );
}

/** A deleted file: red, strike-through, not clickable — there is nothing on
 *  disk to preview (the daemon would 404). */
function DeletedFileRow({ name }: { name: string }) {
  return (
    <div
      title="已从磁盘删除"
      className="flex items-center gap-1 rounded-sm px-1 py-0.5 text-[13px] text-danger"
    >
      <span className="w-3 shrink-0" />
      <FileMinus size={13} className="shrink-0" />
      <span className="truncate line-through">{name}</span>
    </div>
  );
}
