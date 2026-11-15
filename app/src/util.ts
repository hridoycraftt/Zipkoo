import type { Lang } from "./i18n";

const BN_DIGITS = "০১২৩৪৫৬৭৮৯";
export const toBn = (s: string) => s.replace(/\d/g, (d) => BN_DIGITS[Number(d)]);

export function human(n: number, lang: Lang = "en"): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  const s = i === 0 ? `${n} B` : `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
  return lang === "bn" ? toBn(s) : s;
}

export const num = (n: number, lang: Lang) => (lang === "bn" ? toBn(n.toLocaleString("en-US")) : n.toLocaleString("en-US"));

export const baseName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
export const dirName = (p: string) => {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i > 0 ? p.slice(0, i) : p;
};
export const sep = (p: string) => (p.includes("\\") ? "\\" : "/");

/** "Photos.7z" -> "Photos", "a.tar.zst.001" -> "a" */
export function stripArchiveExt(name: string): string {
  return name.replace(/\.\d{3}$/, "").replace(/\.(tar\.(gz|zst|bz2|xz)|tgz|tzst|tbz2|txz|zip|7z|rar|tar)$/i, "");
}

export const uid = () => Math.random().toString(36).slice(2, 10);

const IMG = /\.(png|jpe?g|gif|webp|bmp|svg|avif)$/i;
const TXT = /\.(txt|md|csv|json|xml|html?|css|js|ts|tsx|jsx|py|rs|go|java|c|cpp|h|sh|yml|yaml|toml|ini|log|sql)$/i;
export const previewKind = (name: string): "image" | "text" | "pdf" | "none" =>
  IMG.test(name) ? "image" : TXT.test(name) ? "text" : /\.pdf$/i.test(name) ? "pdf" : "none";

export const mimeFor = (name: string) => {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  return ({ png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", bmp: "image/bmp", svg: "image/svg+xml", avif: "image/avif", pdf: "application/pdf" } as Record<string, string>)[ext] ?? "application/octet-stream";
};
