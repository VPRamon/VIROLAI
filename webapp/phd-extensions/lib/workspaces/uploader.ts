/**
 * Mixed manifest+schedule uploader for `/workspaces/:id`.
 *
 * Accepts loose `File`s as well as DataTransferItems from a folder
 * drop, recursively traverses directories using `webkitGetAsEntry()`,
 * classifies every `.json` payload by inspecting its content, and
 * dispatches batched POSTs to the workspaces API. Progress is reported
 * per file through a callback so the UI can render a queue table.
 */
import {
  WorkspacesApiError,
  addManifestBatch,
  ingestScheduleBatch,
} from './api';

// ── Types ──────────────────────────────────────────────────────────────────

export type QueueItemStatus =
  | 'pending'
  | 'classifying'
  | 'uploading'
  | 'created'
  | 'deduped'
  | 'error'
  | 'skipped';

export type QueueItemKind = 'manifest' | 'schedule' | 'unknown';

export interface QueueItem {
  id: string;
  path: string;
  size: number;
  kind: QueueItemKind;
  status: QueueItemStatus;
  message?: string;
  manifestId?: string;
}

export interface UploadOptions {
  workspaceId: string;
  includeSchedules?: boolean;
  manifestChunkSize?: number;
  scheduleChunkSize?: number;
  onUpdate: (items: QueueItem[]) => void;
}

// ── Public API ─────────────────────────────────────────────────────────────

/** Convert a heterogeneous user input into a flat list of `File`s. */
export async function collectFiles(
  input: FileList | File[] | DataTransferItemList,
): Promise<File[]> {
  if (input instanceof FileList) return filterJson(Array.from(input));
  if (Array.isArray(input)) return filterJson(input);

  const files: File[] = [];
  const entries: FileSystemEntry[] = [];
  for (let i = 0; i < input.length; i++) {
    const item = input[i];
    if (item.kind !== 'file') continue;
    const entry = item.webkitGetAsEntry?.();
    if (entry) {
      entries.push(entry);
    } else {
      const f = item.getAsFile();
      if (f) files.push(f);
    }
  }
  for (const entry of entries) {
    await walkEntry(entry, '', files);
  }
  return filterJson(files);
}

/** Top-level orchestrator: classify, then upload in chunks. */
export async function uploadFiles(
  files: File[],
  opts: UploadOptions,
): Promise<QueueItem[]> {
  const items: QueueItem[] = files.map((f, idx) => ({
    id: `q-${idx}-${f.name}`,
    path: relativePath(f),
    size: f.size,
    kind: 'unknown',
    status: 'pending',
  }));
  opts.onUpdate([...items]);

  // Phase 1: classify every file.
  const parsed: { item: QueueItem; payload: unknown }[] = [];
  for (let i = 0; i < files.length; i++) {
    items[i].status = 'classifying';
    opts.onUpdate([...items]);
    try {
      const text = await files[i].text();
      const payload = JSON.parse(text);
      const kind = classify(payload);
      items[i].kind = kind;
      if (kind === 'unknown') {
        items[i].status = 'skipped';
        items[i].message = 'Not a manifest or self-contained schedule';
      } else {
        items[i].status = 'pending';
        parsed.push({ item: items[i], payload });
      }
    } catch (e) {
      items[i].status = 'error';
      items[i].message = e instanceof Error ? e.message : String(e);
    }
    opts.onUpdate([...items]);
  }

  const includeSchedules = opts.includeSchedules ?? true;
  const manifestChunk = opts.manifestChunkSize ?? 50;
  const scheduleChunk = opts.scheduleChunkSize ?? 10;

  const manifests = parsed.filter((p) => p.item.kind === 'manifest');
  const schedules = parsed.filter((p) => p.item.kind === 'schedule');

  // Phase 2a: upload manifests.
  for (const chunk of chunked(manifests, manifestChunk)) {
    chunk.forEach((p) => (p.item.status = 'uploading'));
    opts.onUpdate([...items]);
    try {
      const res = await addManifestBatch(
        opts.workspaceId,
        chunk.map((p) => ({ manifest: p.payload as object })),
      );
      res.results.forEach((r, idx) => {
        const it = chunk[idx].item;
        if (r.ok) {
          it.status = r.created ? 'created' : 'deduped';
          it.manifestId = r.manifest.manifest_id;
        } else {
          it.status = 'error';
          it.message = r.error.message;
        }
      });
    } catch (e) {
      const msg = e instanceof WorkspacesApiError ? e.message : String(e);
      chunk.forEach((p) => {
        p.item.status = 'error';
        p.item.message = msg;
      });
    }
    opts.onUpdate([...items]);
  }

  // Phase 2b: upload schedules (or skip).
  if (!includeSchedules) {
    schedules.forEach((p) => {
      p.item.status = 'skipped';
      p.item.message = 'Schedule upload disabled';
    });
    opts.onUpdate([...items]);
  } else {
    for (const chunk of chunked(schedules, scheduleChunk)) {
      chunk.forEach((p) => (p.item.status = 'uploading'));
      opts.onUpdate([...items]);
      try {
        const res = await ingestScheduleBatch(
          opts.workspaceId,
          chunk.map((p) => ({ schedule: p.payload as object })),
        );
        res.results.forEach((r, idx) => {
          const it = chunk[idx].item;
          if (r.ok) {
            it.status = r.created ? 'created' : 'deduped';
            it.manifestId = r.manifest.manifest_id;
          } else {
            it.status = 'error';
            it.message = r.error.message;
          }
        });
      } catch (e) {
        const msg = e instanceof WorkspacesApiError ? e.message : String(e);
        chunk.forEach((p) => {
          p.item.status = 'error';
          p.item.message = msg;
        });
      }
      opts.onUpdate([...items]);
    }
  }

  return items;
}

// ── Internals ──────────────────────────────────────────────────────────────

function filterJson(files: File[]): File[] {
  return files.filter((f) => /\.json$/i.test(f.name));
}

function relativePath(f: File): string {
  // Chrome populates `webkitRelativePath` for folder uploads.
  const wrp = (f as unknown as { webkitRelativePath?: string }).webkitRelativePath;
  return wrp && wrp.length > 0 ? wrp : f.name;
}

function classify(payload: unknown): QueueItemKind {
  if (!payload || typeof payload !== 'object') return 'unknown';
  const obj = payload as Record<string, unknown>;
  if ('manifest_schema_version' in obj && 'manifest_id' in obj) return 'manifest';
  if ('schedule_metadata' in obj && 'schedule_metrics' in obj) return 'schedule';
  return 'unknown';
}

function* chunked<T>(arr: T[], size: number): Generator<T[]> {
  for (let i = 0; i < arr.length; i += size) yield arr.slice(i, i + size);
}

async function walkEntry(
  entry: FileSystemEntry,
  prefix: string,
  out: File[],
): Promise<void> {
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) =>
      (entry as FileSystemFileEntry).file(resolve, reject),
    );
    // Stamp a relative path for downstream display.
    Object.defineProperty(file, 'webkitRelativePath', {
      value: prefix ? `${prefix}/${file.name}` : file.name,
      configurable: true,
    });
    out.push(file);
  } else if (entry.isDirectory) {
    const dir = entry as FileSystemDirectoryEntry;
    const reader = dir.createReader();
    let done = false;
    while (!done) {
      const batch: FileSystemEntry[] = await new Promise((resolve, reject) =>
        reader.readEntries(resolve, reject),
      );
      if (batch.length === 0) {
        done = true;
      } else {
        for (const child of batch) {
          await walkEntry(child, prefix ? `${prefix}/${dir.name}` : dir.name, out);
        }
      }
    }
  }
}
