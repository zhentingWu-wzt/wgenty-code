import { useCallback, useEffect, useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight, FileText, Folder, Loader2 } from "lucide-react";
import { toast } from "sonner";
import type { DaemonClient } from "../../api/client";
import type { FsEntries } from "../../api/types";
import { cn } from "../../lib/utils";
import { useUiStore } from "../../state/uiStore";

/**
 * Workspace file tree (design doc §1.4) — read-only browsing of one workspace
 * root (main checkout or a linked worktree task), mounted inside ProjectTree's
 * per-task「文件」group node.
 *
 * Lazy loading: the root listing is fetched on mount (the group node is
 * default-collapsed, so "expand" == mount == the first `listEntries`), each
 * sub-directory fetches on its own first expand, and listings are cached by
 * directory path — collapsing never refetches.
 */

/** Daemon-side listing cap (src/daemon/workspace_files.rs). */
const LIST_LIMIT = 2000;

/** Generated dirs that are usually build output / deps: greyed out and stay
 *  collapsed by default (expanding is still allowed). */
const MUTED_DIR_NAMES = new Set(["target", "node_modules", "dist"]);

export function isMutedDir(name: string): boolean {
  return MUTED_DIR_NAMES.has(name);
}

/** Extensions treated as text for the tab's first-guess `kind`. The preview
 *  panel re-checks against the actual fetchFile response, so this only picks
 *  the initial icon/fallback — unknown or non-text extensions guess "binary". */
const TEXT_EXTENSIONS = new Set([
  // systems languages
  "rs", "go", "zig", "py", "rb", "java", "kt", "swift", "scala", "dart",
  "c", "h", "cpp", "hpp", "cc", "hh", "cs", "m", "mm", "php", "lua", "pl", "r",
  // shell
  "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd",
  // web / frontend
  "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte",
  "html", "htm", "css", "scss", "sass", "less",
  // data & config
  "json", "jsonc", "json5", "toml", "yaml", "yml", "xml", "ini", "cfg", "conf",
  "env", "properties", "lock", "sql", "csv", "tsv",
  // docs & text
  "md", "mdx", "txt", "rst", "adoc", "log", "diff", "patch",
]);

/** Common extensionless files that are text (matched lowercased). */
const TEXT_FILENAMES = new Set([
  "makefile", "dockerfile", "license", "licence", "readme", "changelog",
  "notice", "authors", "contributors", "codeowners",
  ".gitignore", ".gitattributes", ".gitmodules", ".dockerignore",
  ".editorconfig", ".env", ".npmrc", ".nvmrc", ".tool-versions",
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

/** Path of `abs` relative to workspace `root` ("" at the root itself).
 *  Falls back to `abs` unchanged if it is somehow not under the root. */
export function relativeTo(root: string, abs: string): string {
  if (root === "/") return abs.startsWith("/") ? abs.slice(1) : abs;
  const trimmed = root.endsWith("/") ? root.slice(0, -1) : root;
  if (abs === trimmed) return "";
  if (abs.startsWith(`${trimmed}/`)) return abs.slice(trimmed.length + 1);
  return abs;
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [client, workspaceRoot],
  );

  // The「文件」group node keeps FileTree unmounted while collapsed, so this
  // mount effect IS the "first expand sends the first listEntries" gate.
  useEffect(() => {
    void load(workspaceRoot);
  }, [load, workspaceRoot]);

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
        {listing.entries.map((e) =>
          e.is_dir ? (
            <DirNode key={e.name} dir={joinPath(parent, e.name)} name={e.name} />
          ) : (
            <FileRow key={e.name} dir={parent} name={e.name} />
          ),
        )}
        {listing.truncated && (
          <div className="truncate px-1 py-0.5 text-[11px] text-muted-foreground">
            已截断，仅显示前 {LIST_LIMIT} 项
          </div>
        )}
      </>
    );
  };

  const DirNode = ({ dir, name }: { dir: string; name: string }) => {
    const isOpen = expanded.has(dir);
    const muted = isMutedDir(name);
    return (
      <div>
        <div className="group flex items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent">
          <button
            type="button"
            aria-expanded={isOpen}
            title={dir}
            onClick={() => toggleDir(dir)}
            className="flex min-w-0 flex-1 items-center gap-1 text-left text-[13px]"
          >
            {isOpen ? <ChevronDown size={12} className="shrink-0" /> : <ChevronRight size={12} className="shrink-0" />}
            <Folder size={13} className={cn("shrink-0", muted && "text-muted-foreground")} />
            <span className={cn("truncate", muted && "text-muted-foreground")}>{name}</span>
          </button>
        </div>
        {isOpen && <div className="ml-4 flex flex-col gap-0.5">{renderDirBody(dir)}</div>}
      </div>
    );
  };

  const FileRow = ({ dir, name }: { dir: string; name: string }) => {
    const absPath = joinPath(dir, name);
    return (
      <div className="flex items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent">
        <button
          type="button"
          title={absPath}
          onClick={() => openFile(dir, name)}
          className="flex min-w-0 flex-1 items-center gap-1 text-left text-[13px]"
        >
          {/* Chevron-width spacer keeps file names aligned with dir names. */}
          <span className="w-3 shrink-0" />
          <FileText size={13} className="shrink-0" />
          <span className="truncate">{name}</span>
        </button>
      </div>
    );
  };

  return (
    <div className="flex flex-col gap-0.5" data-testid="file-tree" data-root={workspaceRoot}>
      {renderDirBody(workspaceRoot)}
    </div>
  );
}
