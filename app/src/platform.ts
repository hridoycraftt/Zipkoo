// Which desktop Zipkoo runs on. The look follows it: Apple's conventions on the Mac,
// Fluent (Windows 11) on Windows, the plain Daylight theme elsewhere.
export type Platform = "mac" | "windows" | "linux";

function detect(): Platform {
  try {
    // For screenshots and tests: localStorage "zk-platform" forces a look.
    const forced = localStorage.getItem("zk-platform");
    if (forced === "mac" || forced === "windows" || forced === "linux") return forced;
  } catch {
    /* storage blocked: detect normally */
  }
  const ua = navigator.userAgent;
  if (/Mac OS X|Macintosh/.test(ua)) return "mac";
  if (/Windows/.test(ua)) return "windows";
  return "linux";
}

export const platform: Platform = detect();
document.documentElement.dataset.platform = platform;
