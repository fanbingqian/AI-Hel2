import { useEffect, useState, useRef } from "react";
import styles from "./SplashScreen.module.css";

interface Props {
  onComplete: () => void;
}

export function SplashScreen({ onComplete }: Props) {
  const [fadeOut, setFadeOut] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    timerRef.current = setTimeout(() => {
      setFadeOut(true);
      setTimeout(onComplete, 600);
    }, 5000);

    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [onComplete]);

  const handleSkip = () => {
    if (timerRef.current) clearTimeout(timerRef.current);
    setFadeOut(true);
    setTimeout(onComplete, 600);
  };

  return (
    <div className={`${styles.overlay} ${fadeOut ? styles.fadeOut : ""}`}>
      <div className={styles.logoWrap}>
        <div className={styles.logo}>
          AI-<span>Hel2</span>
        </div>
        <div className={styles.glow} />
      </div>
      <div className={styles.progress}>
        <div className={styles.progressBar} />
      </div>
      <button className={styles.skipBtn} onClick={handleSkip}>
        跳过动画
      </button>
    </div>
  );
}
