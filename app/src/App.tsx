import { useCallback, useEffect, useMemo, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  asUiError, cancelJob, extractArchive, inspectArchive, isArchive, launchPaths, onProgress, packFiles,
  type ArchiveInfo, type PackRequest, type Progress, type UiError,
} from "./api";
import { makeT, type Lang } from "./i18n";
import { baseName, dirName, human, num, sep, stripArchiveExt, uid } from "./util";
import { Mark } from "./components/Mark";
import { Home } from "./components/Home";
import { ArchiveView } from "./components/ArchiveView";
import { PackSheet } from "./components/PackSheet";
import { PasswordDialog, ProgressOverlay, Toast, type ToastData } from "./components/Overlays";

const store = {
  get<T>(k: string, d: T): T {
    try {
      const v = localStorage.getItem(k);
      return v ? (JSON.parse(v) as T) : d;
    } catch {
      return d;
    }
  },
  set(k: string, v: unknown) {
    try {
      localStorage.setItem(k, JSON.stringify(v));
    } catch {
      /* storage unavailable */
    }
  },
};

type Job = { id: string; kind: "pack" | "extract"; progress: Progress | null };

export default function App() {
  const [lang, setLang] = useState<Lang>(() => store.get<Lang>("lang", navigator.language.startsWith("bn") ? "bn" : "en"));
  const t = useMemo(() => makeT(lang), [lang]);
  const [recent, setRecent] = useState<string[]>(() => store.get("recent", []));
  const [archive, setArchive] = useState<{ info: ArchiveInfo; password?: string } | null>(null);
  const [packInputs, setPackInputs] = useState<string[] | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [toast, setToast] = useState<ToastData | null>(null);
  const [pwPrompt, setPwPrompt] = useState<{ path: string; error?: string } | null>(null);
  const [dragging, setDragging] = useState(false);
  const [skipPrograms, setSkipPrograms] = useState(true);

  useEffect(() => {
    document.documentElement.lang = lang;
    store.set("lang", lang);
  }, [lang]);

  const remember = useCallback((p: string) => {
    setRecent((r) => {
      const next = [p, ...r.filter((x) => x !== p)].slice(0, 12);
      store.set("recent", next);
      return next;
    });
  }, []);

  const errorText = useCallback(
    (e: UiError) =>
      e.code === "wrong_password" ? t("wrongPw")
      : e.code === "too_large" ? t("tooLarge")
      : e.code === "unsupported" ? t("unsupported")
      : e.code === "damaged" ? `${t("damaged")} ${e.message}`
      : e.message,
    [t],
  );

  const openPath = useCallback(
    async (path: string, password?: string) => {
      try {
        const info = await inspectArchive(path, password);
        if (info.encrypted && !password) {
          setPwPrompt({ path });
          return;
        }
        setPwPrompt(null);
        setArchive({ info, password });
        setSkipPrograms(info.executables.length > 0);
        remember(path);
      } catch (err) {
        const e = asUiError(err);
        if (e.code === "password_required" || (e.code === "wrong_password" && !password)) setPwPrompt({ path });
        else if (e.code === "wrong_password") setPwPrompt({ path, error: t("wrongPw") });
        else setToast({ kind: "err", text: errorText(e) });
      }
    },
    [remember, t, errorText],
  );

  const handlePaths = useCallback(
    (paths: string[]) => {
      if (!paths.length) return;
      if (paths.length === 1 && isArchive(paths[0])) openPath(paths[0]);
      else setPackInputs(paths);
    },
    [openPath],
  );

  // Progress events, drag and drop, and "Open with Zipkoo" launch paths.
  useEffect(() => {
    const un = onProgress((id, p) => setJob((j) => (j && j.id === id ? { ...j, progress: p } : j)));
    const dd = getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") setDragging(true);
      else if (p.type === "leave") setDragging(false);
      else if (p.type === "drop") {
        setDragging(false);
        handlePaths(p.paths);
      }
    });
    launchPaths().then(handlePaths).catch(() => {});
    return () => {
      un.then((f) => f());
      dd.then((f) => f());
    };
  }, [handlePaths]);

  const pick = async (kind: "files" | "folder" | "archive") => {
    const res = await openDialog({
      multiple: kind === "files",
      directory: kind === "folder",
      filters: kind === "archive" ? [{ name: "Archives", extensions: ["zip", "7z", "rar", "tar", "gz", "tgz", "zst", "tzst", "bz2", "xz", "001"] }] : undefined,
    });
    if (!res) return;
    const paths = Array.isArray(res) ? res : [res];
    if (kind === "archive") openPath(paths[0]);
    else setPackInputs(paths);
  };

  const startPack = async (req: PackRequest) => {
    setPackInputs(null);
    const id = uid();
    setJob({ id, kind: "pack", progress: null });
    try {
      const r = await packFiles(id, req);
      const savedBytes = Math.max(0, r.input_bytes - r.output_bytes);
      remember(r.outputs[0]);
      setToast({
        kind: "ok",
        text: savedBytes > 0 ? t("zippedToast", { saved: human(savedBytes, lang) }) : t("zippedToastNoSave", { n: num(r.files, lang) }),
        sub: r.stored_as_is ? t("storedNote", { n: num(r.stored_as_is, lang) }) : undefined,
        action: { label: t("showInFolder"), run: () => revealItemInDir(r.outputs[0]) },
      });
    } catch (err) {
      const e = asUiError(err);
      setToast(e.code === "cancelled" ? { kind: "info", text: t("cancelled") } : { kind: "err", text: errorText(e) });
    } finally {
      setJob(null);
    }
  };

  const startExtract = async (only: string[], chooseDest: boolean) => {
    if (!archive) return;
    const { info, password } = archive;
    let parent = dirName(info.path);
    if (chooseDest) {
      const d = await openDialog({ directory: true, defaultPath: parent });
      if (!d || Array.isArray(d)) return;
      parent = d;
    }
    const dest = `${parent}${sep(info.path)}${stripArchiveExt(baseName(info.path))}`;
    const id = uid();
    setJob({ id, kind: "extract", progress: null });
    try {
      const r = await extractArchive(id, { archive: info.path, dest, password: password ?? null, overwrite: false, allowPrograms: !skipPrograms, only });
      setToast({
        kind: "ok",
        text: t("extractedToast", { n: num(r.files, lang), dest: baseName(r.dest) }),
        sub: r.skipped.length ? `${t("skippedNote", { n: num(r.skipped.length, lang) })}: ${r.skipped.slice(0, 3).map(([n, why]) => `${baseName(n)} (${why})`).join(", ")}` : undefined,
        action: { label: t("showInFolder"), run: () => revealItemInDir(r.dest) },
      });
    } catch (err) {
      const e = asUiError(err);
      setToast(e.code === "cancelled" ? { kind: "info", text: t("cancelled") } : { kind: "err", text: errorText(e) });
    } finally {
      setJob(null);
    }
  };

  return (
    <div className={`app ${dragging ? "dragging" : ""}`}>
      <nav className="side">
        <div className="brand">
          <Mark size={28} />
          <span className="wordmark">zipkoo</span>
        </div>
        <button className={`nav ${!archive ? "on" : ""}`} onClick={() => setArchive(null)}>{t("home")}</button>
        <h2 className="eyebrow">{t("recent")}</h2>
        <div className="side-recent">
          {recent.slice(0, 8).map((p) => (
            <button key={p} className={`nav small ${archive?.info.path === p ? "on" : ""}`} title={p} onClick={() => openPath(p)}>
              {baseName(p)}
            </button>
          ))}
        </div>
        <div className="side-foot">
          <button className="btn ghost small" onClick={() => setLang(lang === "en" ? "bn" : "en")}>{t("lang")}</button>
          <span className="muted small">{t("tagline")}</span>
        </div>
      </nav>

      <main className="content">
        {archive ? (
          <ArchiveView
            t={t}
            lang={lang}
            info={archive.info}
            password={archive.password}
            skipPrograms={skipPrograms}
            setSkipPrograms={setSkipPrograms}
            onExtract={startExtract}
            onClose={() => setArchive(null)}
          />
        ) : (
          <Home t={t} recent={recent} onPackFiles={() => pick("files")} onPackFolder={() => pick("folder")} onOpen={() => pick("archive")} onOpenPath={openPath} />
        )}
      </main>

      {dragging && (
        <div className="drag-veil" aria-hidden="true">
          <Mark size={64} />
          <span>{t("dragHere")}</span>
        </div>
      )}
      {packInputs && <PackSheet t={t} lang={lang} inputs={packInputs} onCancel={() => setPackInputs(null)} onStart={startPack} />}
      {pwPrompt && (
        <PasswordDialog t={t} name={baseName(pwPrompt.path)} error={pwPrompt.error} onSubmit={(pw) => openPath(pwPrompt.path, pw)} onCancel={() => setPwPrompt(null)} />
      )}
      {job && <ProgressOverlay t={t} lang={lang} title={job.kind === "pack" ? t("working") : t("extracting")} progress={job.progress} onCancel={() => cancelJob(job.id)} />}
      {toast && <Toast toast={toast} onDone={() => setToast(null)} />}
    </div>
  );
}
