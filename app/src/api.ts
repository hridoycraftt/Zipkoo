// Typed wrappers around the Rust commands in src-tauri/src/lib.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Format = "zip" | "seven-z" | "tar" | "tar-gz" | "tar-zst" | "tar-bz2" | "tar-xz" | "rar" | "gz" | "xz" | "bz2" | "zst" | "iso" | "cab" | "lzh";
export type Level = "fast" | "normal" | "max";

export interface Entry {
  name: string;
  size: number;
  packed: number | null;
  is_dir: boolean;
  encrypted: boolean;
  executable: boolean;
}

export interface ArchiveInfo {
  path: string;
  format: Format;
  archive_size: number;
  total_size: number;
  file_count: number;
  encrypted: boolean;
  executables: string[];
  entries: Entry[];
}

export interface Progress {
  done_bytes: number;
  total_bytes: number;
  current: string;
}

export interface PackReport {
  outputs: string[];
  files: number;
  input_bytes: number;
  output_bytes: number;
  stored_as_is: number;
}

export interface ExtractReport {
  dest: string;
  files: number;
  bytes: number;
  skipped: [string, string][];
}

export interface UiError {
  code: "password_required" | "wrong_password" | "too_large" | "no_space" | "no_server" | "no_access" | "cancelled" | "unsupported" | "damaged" | "error";
  message: string;
}

export function asUiError(e: unknown): UiError {
  if (e && typeof e === "object" && "code" in e && (e as UiError).code === "no_space") {
    const m = /needs (\d+) bytes, but only (\d+)/.exec((e as UiError).message);
    if (m) return { code: "no_space", message: `${m[1]}|${m[2]}` };
  }
  if (e && typeof e === "object" && "code" in e) return e as UiError;
  return { code: "error", message: String(e) };
}

export const FORMAT_LABEL: Record<Format, string> = {
  zip: "ZIP", "seven-z": "7Z", tar: "TAR", "tar-gz": "TAR.GZ", "tar-zst": "TAR.ZST",
  "tar-bz2": "TAR.BZ2", "tar-xz": "TAR.XZ", rar: "RAR",
  gz: "GZ", xz: "XZ", bz2: "BZ2", zst: "ZST", iso: "ISO", cab: "CAB", lzh: "LZH",
};
export const FORMAT_EXT: Record<"zip" | "seven-z" | "tar-zst", string> = { zip: "zip", "seven-z": "7z", "tar-zst": "tar.zst" };

export const inspectArchive = (path: string, password?: string) =>
  invoke<ArchiveInfo>("inspect_archive", { path, password: password ?? null });

export const peekFile = async (path: string, name: string, password?: string) =>
  new Uint8Array(await invoke<ArrayBuffer>("peek_file", { path, name, password: password ?? null }));

export interface PackRequest {
  inputs: string[];
  output: string;
  format: "zip" | "seven-z" | "tar-zst";
  level: Level;
  smart: boolean;
  password: string | null;
  hideNames: boolean;
  splitSize: number | null;
  sfx: boolean;
  shrinkPhotos?: boolean;
  repairPercent?: number | null;
}

export const packFiles = (job: string, req: PackRequest) => invoke<PackReport>("pack_files", { job, req });

export interface ExtractRequest {
  archive: string;
  dest: string;
  password: string | null;
  overwrite: boolean;
  allowPrograms: boolean;
  only: string[];
  strip?: string | null;
  onConflict?: "skip" | "overwrite" | "rename";
}

export const extractArchive = (job: string, req: ExtractRequest) => invoke<ExtractReport>("extract_archive", { job, req });
export const cancelJob = (job: string) => invoke<void>("cancel_job", { job });
export const hashFile = (path: string) => invoke<string>("hash_file", { path });
export interface ShellAction {
  action: "zip-here" | "lock" | "add" | "sfx" | "extract-here" | "open" | string;
  paths: string[];
}
export const takeShellActions = () => invoke<ShellAction[]>("take_shell_actions");
export const onShellActions = (cb: () => void) => listen("shell-actions", cb);
export const sfxAvailable = () => invoke<boolean>("sfx_available");
export const suggestPackOutput = (inputs: string[], ext: string) => invoke<string | null>("suggest_pack_output", { inputs, ext });
export const suggestExtractDest = (path: string, password?: string) =>
  invoke<{ dest: string; result: string; strip: string | null }>("suggest_extract_dest", { path, password: password ?? null });
export interface ShellStatus { supported: boolean; installed: boolean }
export const shellStatus = () => invoke<ShellStatus>("shell_status");
export const shellSet = (enabled: boolean) => invoke<ShellStatus>("shell_set", { enabled });

export function onProgress(cb: (job: string, p: Progress) => void): Promise<UnlistenFn> {
  return listen<{ job: string; progress: Progress }>("job-progress", (e) => cb(e.payload.job, e.payload.progress));
}

const ARCHIVE_RE = /\.(zip|zipx|7z|rar|tar|tgz|tzst|tbz2|txz|gz|zst|bz2|xz|001|iso|cab|lzh|lha)$/i;
export const ARCHIVE_EXTS = ["zip", "zipx", "7z", "rar", "tar", "gz", "tgz", "zst", "tzst", "bz2", "tbz2", "xz", "txz", "001", "iso", "cab", "lzh", "lha", "exe"];
export const isArchive = (p: string) => ARCHIVE_RE.test(p);

// ---------- Features 6–12 ----------
export interface UpdateReport { version: number; added: number; changed: number; removed: number; unchanged: number; bytes_packed: number }
export interface Version { version: number; time: number; added: string[]; changed: string[]; removed: string[] }
export interface Health { outdated: boolean; blocks: number; damaged: number; unrecoverable: number; size_ok: boolean; repaired: number }
export interface DupGroup { size: number; files: string[]; wasted: number }
export interface CloudFolder { provider: string; label: string; path: string }
export interface FolderItem { path: string; size: number; modified: number }
export interface Hit { archive: string; name: string; size: number }
export interface IndexStats { archives: number; files: number; indexed_now: number }
export interface ShareRecord { url: string; id: string; deleteToken: string; expiresAt: number; maxDownloads: number | null; password: boolean; name: string; size: number; created: number; server: string }
export interface Watch { id: string; folder: string; dest: string; intervalMinutes: number; keep: number; repair: boolean; paused: boolean; lastRun: number | null; lastResult: string | null; lastOk: boolean }
export interface ConvertResult { source: string; output: string | null; error: string | null; output_bytes: number }

export const convertArchives = (jobId: string, archives: string[], format: "zip" | "seven-z" | "tar-zst", password?: string) =>
  invoke<ConvertResult[]>("convert_archives", { jobId, archives, format, password: password ?? null });
export const updateArchive = (jobId: string, archive: string, inputs: string[], password?: string, removeMissing = false) =>
  invoke<UpdateReport>("update_archive", { jobId, archive, inputs, password: password ?? null, removeMissing });
export const archiveHistory = (archive: string) => invoke<{ versions: Version[] }>("archive_history", { archive });
export const healStatus = (archive: string) => invoke<{ has_repair_data: boolean }>("heal_status", { archive });
export const healCreate = (jobId: string, archive: string, percent = 10) => invoke<{ path: string; size: number }>("heal_create", { jobId, archive, percent });
export const healCheck = (jobId: string, archive: string, fix: boolean) => invoke<Health>("heal_check", { jobId, archive, fix });
export const findDuplicates = (inputs: string[]) => invoke<DupGroup[]>("find_duplicates", { inputs });
export const cloudFolders = () => invoke<CloudFolder[]>("cloud_folders");
export const listArchives = (folder: string) => invoke<FolderItem[]>("list_archives", { folder });
export const indexRefresh = (extra: string[]) => invoke<IndexStats>("index_refresh", { extra });
export const searchArchives = (query: string) => invoke<Hit[]>("search_archives", { query });
export const shareFile = (jobId: string, path: string, server: string, hours: number, downloads: number | null, password: string | null) =>
  invoke<ShareRecord>("share_file", { jobId, path, server, hours, downloads, password });
export const sharesList = () => invoke<ShareRecord[]>("shares_list");
export const shareDelete = (id: string) => invoke<void>("share_delete", { id });
export const shareDownload = (jobId: string, link: string, dest: string, password?: string) =>
  invoke<string>("share_download", { jobId, link, dest, password: password ?? null });
export const watchesList = () => invoke<Watch[]>("watches_list");
export const watchSave = (watch: Watch) => invoke<Watch[]>("watch_save", { watch });
export const watchRemove = (id: string) => invoke<Watch[]>("watch_remove", { id });
export const watchRunNow = (id: string) => invoke<Watch[]>("watch_run_now", { id });
export const onEvent = (name: string, cb: () => void) => listen(name, cb);

export type Change =
  | { kind: "add"; inputs: string[]; folder: string; replace: boolean }
  | { kind: "delete"; names: string[] }
  | { kind: "rename"; from: string; to: string };
export interface EditReport { added: number; replaced: number; deleted: number; renamed: number; skipped: string[] }
export const editArchive = (jobId: string, archive: string, change: Change, password?: string) =>
  invoke<EditReport>("edit_archive", { jobId, archive, change, password: password ?? null });
export const openEntry = (archive: string, name: string, password?: string) => invoke<string>("open_entry", { archive, name, password: password ?? null });
export const prepareDrag = (archive: string, names: string[], password?: string) =>
  invoke<{ items: string[]; icon: string }>("prepare_drag", { archive, names, password: password ?? null });
export const EDITABLE: Format[] = ["zip", "seven-z", "tar-zst"];

export const extractConflicts = (q: { archive: string; dest: string; password?: string | null; only?: string[]; strip?: string | null }) =>
  invoke<string[]>("extract_conflicts", { q: { ...q, password: q.password ?? null } });
export interface TestReport { files: number; bytes: number; errors: [string, string][] }
/** null when Zipkoo may use `dir`; otherwise the existing folder the person must choose. */
export const folderAccess = (dir: string, write: boolean) => invoke<string | null>("folder_access", { dir, write });
export const folderGrant = (dir: string) => invoke<void>("folder_grant", { dir });
export interface SalvageReport { output: string; recovered: number; bytes: number; lost: [string, string][] }
export const salvageZip = (jobId: string, archive: string) => invoke<SalvageReport>("salvage_zip", { jobId, archive });
export const testArchive = (jobId: string, archive: string, password?: string) => invoke<TestReport>("test_archive", { jobId, archive, password: password ?? null });

export interface AppInfo { version: string; flavor: "direct" | "appstore"; updater_ready: boolean; os: string }
export const appInfo = () => invoke<AppInfo>("app_info");
export const trashPath = (path: string) => invoke<void>("trash_path", { path });

export interface SavedPassword { id: string; label: string; created: number }
export const passwordsList = () => invoke<SavedPassword[]>("passwords_list");
export const passwordAdd = (label: string, password: string, archive?: string) => invoke<SavedPassword[]>("password_add", { label, password, archive: archive ?? null });
export const passwordRemove = (id: string) => invoke<SavedPassword[]>("password_remove", { id });
export const passwordFind = (path: string) => invoke<string | null>("password_find", { path });

export interface ScanResult { engine: string; clean: boolean; threats: string[] }
export const scanAvailable = () => invoke<string | null>("scan_available");
export const virusScan = (path: string) => invoke<ScanResult>("virus_scan", { path });
