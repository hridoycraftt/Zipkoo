// Typed wrappers around the Rust commands in src-tauri/src/lib.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Format = "zip" | "seven-z" | "tar" | "tar-gz" | "tar-zst" | "tar-bz2" | "tar-xz" | "rar";
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
  code: "password_required" | "wrong_password" | "too_large" | "no_space" | "cancelled" | "unsupported" | "damaged" | "error";
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
}

export const packFiles = (job: string, req: PackRequest) => invoke<PackReport>("pack_files", { job, req });

export interface ExtractRequest {
  archive: string;
  dest: string;
  password: string | null;
  overwrite: boolean;
  allowPrograms: boolean;
  only: string[];
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
  invoke<{ dest: string; result: string }>("suggest_extract_dest", { path, password: password ?? null });
export interface ShellStatus { supported: boolean; installed: boolean }
export const shellStatus = () => invoke<ShellStatus>("shell_status");
export const shellSet = (enabled: boolean) => invoke<ShellStatus>("shell_set", { enabled });

export function onProgress(cb: (job: string, p: Progress) => void): Promise<UnlistenFn> {
  return listen<{ job: string; progress: Progress }>("job-progress", (e) => cb(e.payload.job, e.payload.progress));
}

const ARCHIVE_RE = /\.(zip|7z|rar|tar|tgz|tzst|tbz2|txz|gz|zst|bz2|xz|001)$/i;
export const isArchive = (p: string) => ARCHIVE_RE.test(p);
