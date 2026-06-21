import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import styles from "./UpdateDialog.module.css";

interface UpdateProgress {
  downloaded: number;
  total: number;
  percent: number;
}

export function UpdateDialog({ onClose }: { onClose: () => void }) {
  const [step, setStep] = useState<"checking" | "available" | "downloading" | "done" | "error">("checking");
  const [currentVer, setCurrentVer] = useState("");
  const [latestVer, setLatestVer] = useState("");
  const [notes, setNotes] = useState("");
  const [progress, setProgress] = useState(0);
  const [downloaded, setDownloaded] = useState("0 MB");
  const [totalSize, setTotalSize] = useState("0 MB");
  const [errorMsg, setErrorMsg] = useState("");

  useEffect(() => {
    (async () => {
      try {
        const info: any = await invoke("check_update");
        setCurrentVer(info.current_version || "?");
        setLatestVer(info.version || "?");
        setNotes(info.notes || "");
        setStep("available");
      } catch (e: any) {
        setStep("checking"); // stay on checking — no update needed or network error
        setErrorMsg(typeof e === "string" ? e : e?.message || "检查失败");
        setTimeout(onClose, 2000);
      }
    })();

    const unlisten = listen<UpdateProgress>("update:progress", (evt) => {
      setProgress(evt.payload.percent);
      setDownloaded(formatBytes(evt.payload.downloaded));
      setTotalSize(formatBytes(evt.payload.total));
    });
    const unlistenDone = listen("update:done", () => {
      setStep("done");
    });
    return () => { unlisten.then(f => f()); unlistenDone.then(f => f()); };
  }, []);

  const handleDownload = async () => {
    setStep("downloading");
    try {
      await invoke("download_update", { url: `https://github.com/fanbingqian/AI-Hel2/releases/download/v${latestVer}/AI-Hel2_${latestVer}_x64-setup.exe` });
    } catch (e: any) {
      setErrorMsg(typeof e === "string" ? e : e?.message || "下载失败");
      setStep("error");
    }
  };

  return (
    <div className={styles.overlay} onClick={onClose}>
      <div className={styles.dialog} onClick={e => e.stopPropagation()}>
        <h3 className={styles.title}>软件更新</h3>

        {step === "checking" && (
          <div className={styles.body}>
            <p>正在检查更新...</p>
            {errorMsg && <p className={styles.hint}>{errorMsg}</p>}
          </div>
        )}

        {step === "available" && (
          <div className={styles.body}>
            <div className={styles.verRow}>
              <span>当前版本</span><span className={styles.ver}>{currentVer}</span>
            </div>
            <div className={styles.verRow}>
              <span>最新版本</span><span className={styles.verNew}>{latestVer}</span>
            </div>
            {notes && <p className={styles.hint}>{notes.slice(0, 200)}</p>}
            <button className={styles.btn} onClick={handleDownload}>下载更新</button>
          </div>
        )}

        {step === "downloading" && (
          <div className={styles.body}>
            <div className={styles.progressBar}>
              <div className={styles.progressFill} style={{ width: `${Math.min(progress, 100)}%` }} />
            </div>
            <p className={styles.progressText}>
              {progress > 0 ? `${progress.toFixed(0)}%` : "正在连接..."} · {downloaded} / {totalSize}
            </p>
          </div>
        )}

        {step === "done" && (
          <div className={styles.body}>
            <p className={styles.ok}>下载完成，安装程序已启动</p>
            <p className={styles.hint}>关闭 AI-Hel2 后运行安装程序完成更新</p>
          </div>
        )}

        {step === "error" && (
          <div className={styles.body}>
            <p className={styles.err}>{errorMsg}</p>
            <button className={styles.btn} onClick={handleDownload}>重试</button>
          </div>
        )}

        <button className={styles.closeBtn} onClick={onClose}>×</button>
      </div>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
