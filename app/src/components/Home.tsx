import { Mark } from "./Mark";
import type { makeT } from "../i18n";
import { baseName, dirName } from "../util";

interface Props {
  t: ReturnType<typeof makeT>;
  recent: string[];
  onPackFiles: () => void;
  onPackFolder: () => void;
  onOpen: () => void;
  onOpenPath: (p: string) => void;
}

export function Home({ t, recent, onPackFiles, onPackFolder, onOpen, onOpenPath }: Props) {
  return (
    <div className="home">
      <section className="drop-hero">
        <Mark size={72} />
        <h1>{t("dropTitle")}</h1>
        <p className="muted">{t("dropSub")}</p>
        <div className="row gap">
          <button className="btn primary" onClick={onPackFiles}>{t("packFiles")}</button>
          <button className="btn" onClick={onPackFolder}>{t("packFolder")}</button>
          <button className="btn" onClick={onOpen}>{t("openArchive")}</button>
        </div>
      </section>
      <section className="recent">
        <h2 className="eyebrow">{t("recent")}</h2>
        {recent.length === 0 ? (
          <p className="muted small">{t("noRecent")}</p>
        ) : (
          <ul>
            {recent.map((p) => (
              <li key={p}>
                <button className="recent-item" onClick={() => onOpenPath(p)} title={p}>
                  <span className="fname">{baseName(p)}</span>
                  <span className="mono muted small">{dirName(p)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
