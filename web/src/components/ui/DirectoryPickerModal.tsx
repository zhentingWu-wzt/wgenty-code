import { useCallback, useEffect, useMemo, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { ChevronRight, Folder, FolderOpen, CornerUpLeft } from "lucide-react";
import { toast } from "sonner";
import type { DaemonClient } from "../../api/client";
import type { DirEntry } from "../../api/types";
import { cn } from "../../lib/utils";
import { Button } from "./button";

const LAST_DIR_KEY = "wgenty.lastDir";

function loadLastDir(): string | null {
  try {
    return localStorage.getItem(LAST_DIR_KEY);
  } catch {
    return null;
  }
}

function saveLastDir(path: string): void {
  try {
    localStorage.setItem(LAST_DIR_KEY, path);
  } catch {
    /* localStorage unavailable (private mode) - non-fatal. */
  }
}

/** Split an absolute path into clickable breadcrumb segments. */
function segments(path: string): { label: string; path: string }[] {
  // Leading "/" yields an empty first segment on Unix; drop it and render a
  // root "/" entry instead so users can click back to the filesystem root.
  const parts = path.split("/").filter(Boolean);
  const out: { label: string; path: string }[] = [];
  if (path.startsWith("/")) {
    out.push({ label: "/", path: "/" });
  }
  let acc = path.startsWith("/") ? "" : ".";
  for (const part of parts) {
    acc = acc === "/" ? `/${part}` : `${acc}/${part}`;
    out.push({ label: part, path: acc });
  }
  return out;
}

export interface DirectoryPickerModalProps {
  open: boolean;
  client: DaemonClient;
  onOpenChange: (open: boolean) => void;
  /** Called with the selected absolute path when the user confirms. */
  onConfirm: (path: string) => void;
}

export function DirectoryPickerModal({
  open,
  client,
  onOpenChange,
  onConfirm,
}: DirectoryPickerModalProps) {
  const [current, setCurrent] = useState<string>("");
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [parent, setParent] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Editable path box - synced with the browsed dir, but the user can type a
  // full path directly and press Enter to jump there (belt-and-suspenders).
  const [pathInput, setPathInput] = useState<string>("");
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<string | null>(null);

  const load = useCallback(
    async (dir: string) => {
      setLoading(true);
      setError(null);
      setSelected(null);
      try {
        const listing = await client.listDirs(dir || undefined);
        setCurrent(listing.current);
        setEntries(listing.entries);
        setParent(listing.parent);
        setPathInput(listing.current);
        saveLastDir(listing.current);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setEntries([]);
      } finally {
        setLoading(false);
      }
    },
    [client],
  );

  // First open: jump to the last-used dir (or home when none remembered).
  // `load` is a useCallback whose setState calls happen inside it, which the
  // set-state-in-effect rule accepts (same pattern as SessionsBrowserModal).
  // Load the initial listing when the dialog first opens. Fetching remote
  // data on mount/open is a legitimate effect use (React docs endorse it);
  // the setState happens inside `load`, so we disable the local rule here.
  const loadInitial = useCallback(() => {
    if (!open || current) return;
    void load(loadLastDir() ?? "");
  }, [open, current, load]);
  // eslint-disable-next-line react-hooks/set-state-in-effect
  useEffect(loadInitial, [loadInitial]);

  // Wrap onOpenChange so we can clear transient UI state on close (avoids a
  // setState-in-effect lint violation and keeps the next open clean).
  const handleOpenChange = (next: boolean) => {
    if (!next) {
      setFilter("");
      setError(null);
      setSelected(null);
    }
    onOpenChange(next);
  };

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((e) => e.name.toLowerCase().includes(q));
  }, [entries, filter]);

  const goInto = (dir: string) => void load(dir);

  const confirm = () => {
    const target = selected ?? pathInput.trim();
    if (!target) return;
    onConfirm(target);
    onOpenChange(false);
  };

  return (
    <Dialog.Root open={open} onOpenChange={handleOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-black/50" />
        <Dialog.Content
          className="fixed left-1/2 top-1/2 z-50 flex max-h-[80vh] w-[640px] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 rounded-lg border border-border bg-background p-4 shadow-xl"
          onOpenAutoFocus={(e) => e.preventDefault()}
        >
          <Dialog.Title className="text-sm font-semibold">Select project directory</Dialog.Title>

          {/* Editable path box: browse syncs it, but the user may also paste a
              full path and press Enter to jump there. */}
          <div className="flex gap-2">
            <input
              className="min-w-0 flex-1 rounded-md border border-border bg-input px-2 py-1.5 text-sm"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && pathInput.trim()) {
                  void goInto(pathInput.trim());
                }
              }}
              placeholder="Absolute path, e.g. /Users/you/workspace"
              spellCheck={false}
            />
            <Button variant="outline" size="sm" onClick={() => void goInto(pathInput.trim())}>
              Go
            </Button>
          </div>

          {/* Breadcrumb: click any segment to jump back up the tree. */}
          {current && (
            <div className="flex flex-wrap items-center gap-0.5 text-xs text-muted-foreground">
              <button
                type="button"
                disabled={!parent}
                onClick={() => parent && void goInto(parent)}
                className="mr-1 inline-flex items-center gap-1 rounded px-1 py-0.5 hover:bg-accent disabled:opacity-30"
                title="Up to parent"
              >
                <CornerUpLeft size={12} />
              </button>
              {segments(current).map((seg, i) => (
                <span key={seg.path} className="inline-flex items-center">
                  {i > 0 && <ChevronRight size={12} className="mx-0.5 opacity-50" />}
                  <button
                    type="button"
                    onClick={() => void goInto(seg.path)}
                    className="rounded px-1 py-0.5 hover:bg-accent hover:text-foreground"
                  >
                    {seg.label}
                  </button>
                </span>
              ))}
            </div>
          )}

          {/* Filter + list. Hidden dirs render dimmed; double-click enters. */}
          <input
            className="rounded-md border border-border bg-input px-2 py-1.5 text-sm"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter directories…"
            spellCheck={false}
          />

          <div className="min-h-0 flex-1 overflow-auto rounded-md border border-border">
            {loading ? (
              <div className="p-3 text-sm text-muted-foreground">Loading…</div>
            ) : error ? (
              <div className="p-3 text-sm text-destructive">{error}</div>
            ) : filtered.length === 0 ? (
              <div className="p-3 text-sm text-muted-foreground">No subdirectories.</div>
            ) : (
              <ul className="py-1">
                {filtered.map((e) => {
                  const isSelected = selected === e.path;
                  return (
                    <li key={e.path}>
                      <button
                        type="button"
                        onClick={() => setSelected(e.path)}
                        onDoubleClick={() => void goInto(e.path)}
                        className={cn(
                          "flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-accent",
                          isSelected && "bg-accent",
                          e.is_hidden && "opacity-50",
                        )}
                        title={e.path}
                      >
                        {isSelected ? (
                          <FolderOpen size={15} className="shrink-0 text-primary" />
                        ) : (
                          <Folder size={15} className="shrink-0 text-muted-foreground" />
                        )}
                        <span className="truncate">{e.name}</span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>

          <div className="flex items-center justify-between gap-2">
            <span className="truncate text-xs text-muted-foreground" title={selected ?? current}>
              {selected ?? current ?? ""}
            </span>
            <div className="flex gap-2">
              <Dialog.Close asChild>
                <Button variant="outline" size="sm">
                  Cancel
                </Button>
              </Dialog.Close>
              <Button
                size="sm"
                onClick={() => {
                  if (!selected && !pathInput.trim()) {
                    toast.error("Select a directory first");
                    return;
                  }
                  confirm();
                }}
              >
                Select folder
              </Button>
            </div>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
