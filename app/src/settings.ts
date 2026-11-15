// App preferences, kept on this computer (localStorage). Backend options are passed per call.
export type Theme = "system" | "light" | "dark";

export interface Settings {
  theme: Theme;
  defaultFormat: "zip" | "seven-z" | "tar-zst";
  defaultLevel: "fast" | "normal" | "max";
  smart: boolean;
  /** Ask where to extract instead of using the folder next to the archive. */
  extractAsk: boolean;
  openAfterExtract: boolean;
  trashAfterExtract: boolean;
  skipPrograms: boolean;
  autoUpdate: boolean;
}

export const DEFAULTS: Settings = {
  theme: "system",
  defaultFormat: "zip",
  defaultLevel: "normal",
  smart: true,
  extractAsk: false,
  openAfterExtract: false,
  trashAfterExtract: false,
  skipPrograms: true,
  autoUpdate: true,
};

export function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem("settings");
    return raw ? { ...DEFAULTS, ...JSON.parse(raw) } : DEFAULTS;
  } catch {
    return DEFAULTS;
  }
}

export function saveSettings(s: Settings) {
  try {
    localStorage.setItem("settings", JSON.stringify(s));
  } catch {
    /* storage unavailable */
  }
}

/** "system" removes the attribute so the OS setting (prefers-color-scheme) decides. */
export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
}
