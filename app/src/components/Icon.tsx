/** Zipkoo icons: Hugeicons (stroke, rounded), one weight everywhere. */
import { HugeiconsIcon } from "@hugeicons/react";
import {
  Alert02Icon, Cancel01Icon, CheckmarkCircle02Icon, Clock01Icon, ComputerTerminal01Icon, Download04Icon,
  File01Icon, FileZipIcon, Folder01Icon, FolderExportIcon, Globe02Icon, Home09Icon, Image02Icon,
  InformationCircleIcon, Moon02Icon, Note01Icon, Settings02Icon, Sun03Icon, ComputerIcon, PackageIcon, Search01Icon, Shield01Icon, SquareLock02Icon,
} from "@hugeicons/core-free-icons";

const ICONS = {
  home: Home09Icon,
  clock: Clock01Icon,
  search: Search01Icon,
  lock: SquareLock02Icon,
  check: CheckmarkCircle02Icon,
  shield: Shield01Icon,
  folder: Folder01Icon,
  file: File01Icon,
  image: Image02Icon,
  text: Note01Icon,
  app: ComputerTerminal01Icon,
  extract: Download04Icon,
  folderOut: FolderExportIcon,
  pack: PackageIcon,
  zip: FileZipIcon,
  x: Cancel01Icon,
  globe: Globe02Icon,
  info: InformationCircleIcon,
  alert: Alert02Icon,
  settings: Settings02Icon,
  moon: Moon02Icon,
  sun: Sun03Icon,
  computer: ComputerIcon,
};

export type IconName = keyof typeof ICONS;

export function Icon({ name, size = 18, className }: { name: IconName; size?: number; className?: string }) {
  return <HugeiconsIcon icon={ICONS[name]} size={size} strokeWidth={1.6} color="currentColor" className={className} aria-hidden="true" />;
}

/** Pick an icon and a tint for a file name. */
export function fileIcon(name: string, isDir: boolean): { icon: IconName; tint: string } {
  if (isDir) return { icon: "folder", tint: "t-folder" };
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (/^(png|jpe?g|gif|webp|heic|avif|bmp|svg|raw|cr2|nef)$/.test(ext)) return { icon: "image", tint: "t-image" };
  if (/^(exe|msi|bat|cmd|ps1|app|sh|dmg|pkg|jar|scr|vbs)$/.test(ext)) return { icon: "app", tint: "t-app" };
  if (/^(zip|7z|rar|tar|gz|zst|bz2|xz|tgz)$/.test(ext)) return { icon: "zip", tint: "t-arch" };
  if (/^(txt|md|csv|json|xml|html?|css|js|ts|rs|py|log|doc|docx|pdf|rtf|xlsx|xls|pptx)$/.test(ext)) return { icon: "text", tint: "t-doc" };
  return { icon: "file", tint: "t-file" };
}
