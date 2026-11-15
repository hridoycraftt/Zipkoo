/** Zipkoo icon set: 24px grid, 1.6 stroke, round caps. Drawn for this app, one style everywhere. */
const P: Record<string, string> = {
  home: "M4 10.5 12 4l8 6.5V19a1 1 0 0 1-1 1h-4.5v-5.5h-5V20H5a1 1 0 0 1-1-1z",
  clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm0-13v4.5l3 2",
  search: "M10.5 18a7.5 7.5 0 1 0 0-15 7.5 7.5 0 0 0 0 15Zm5.5-2 5 5",
  lock: "M7 10.5V8a5 5 0 0 1 10 0v2.5M5.5 10.5h13a1 1 0 0 1 1 1V20a1 1 0 0 1-1 1h-13a1 1 0 0 1-1-1v-8.5a1 1 0 0 1 1-1Zm6.5 4.5v2.5",
  check: "M5 12.5 10 17.5 19.5 7",
  shield: "M12 3.5 19 6v5.5c0 4.2-2.9 7.6-7 9-4.1-1.4-7-4.8-7-9V6zm0 5v4.5m0 3v.01",
  folder: "M3.5 7.5a2 2 0 0 1 2-2h4l2 2.5h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z",
  file: "M14 3.5H7a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5zm0 0v5h5",
  image: "M5.5 4h13A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-13A1.5 1.5 0 0 1 5.5 4Zm-1.5 12 4.5-4.5 4 4L15 13l5 5M15.5 9.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3Z",
  text: "M14 3.5H7a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5zm0 0v5h5M8.5 13h7m-7 3.5h5",
  app: "M4.5 5.5h15a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1h-15a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1Zm-1 3.5h17M7.5 13l2 1.75-2 1.75m4 0h4",
  extract: "M12 4v10m0 0-4-4m4 4 4-4M5 15v3.5A1.5 1.5 0 0 0 6.5 20h11a1.5 1.5 0 0 0 1.5-1.5V15",
  folderOut: "M3.5 7.5a2 2 0 0 1 2-2h4l2 2.5h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2zm8.5 3.5v5m0 0-2.25-2.25M12 16l2.25-2.25",
  pack: "M12 3.5 20 8v8l-8 4.5L4 16V8zm0 0v17M4 8l8 4.5L20 8",
  x: "M6.5 6.5l11 11m0-11-11 11",
  globe: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm-9-9h18M12 3c2.5 2.5 3.5 5.5 3.5 9s-1 6.5-3.5 9c-2.5-2.5-3.5-5.5-3.5-9s1-6.5 3.5-9Z",
  info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm0-10v5.5m0-8.5v.01",
  alert: "M10.3 4.3 2.9 17.5A2 2 0 0 0 4.6 20.5h14.8a2 2 0 0 0 1.7-3L13.7 4.3a2 2 0 0 0-3.4 0ZM12 9.5v4m0 3v.01",
  split: "M5 4.5h14M5 19.5h14M12 4.5v15M8 9.5l-3 2.5 3 2.5m8-5 3 2.5-3 2.5",
};

export type IconName = keyof typeof P;

export function Icon({ name, size = 18, className }: { name: IconName; size?: number; className?: string }) {
  return (
    <svg className={className} width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={P[name]} />
    </svg>
  );
}

/** Pick a drawn icon and a tint for a file name. */
export function fileIcon(name: string, isDir: boolean): { icon: IconName; tint: string } {
  if (isDir) return { icon: "folder", tint: "t-folder" };
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (/^(png|jpe?g|gif|webp|heic|avif|bmp|svg|raw|cr2|nef)$/.test(ext)) return { icon: "image", tint: "t-image" };
  if (/^(exe|msi|bat|cmd|ps1|app|sh|dmg|pkg|jar|scr|vbs)$/.test(ext)) return { icon: "app", tint: "t-app" };
  if (/^(txt|md|csv|json|xml|html?|css|js|ts|rs|py|log|doc|docx|pdf|rtf|xlsx|xls|pptx)$/.test(ext)) return { icon: "text", tint: "t-doc" };
  return { icon: "file", tint: "t-file" };
}
