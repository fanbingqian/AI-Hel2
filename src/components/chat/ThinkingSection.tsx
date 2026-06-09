import { useState, useRef, useEffect } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ChevronRight, ChevronDown } from "lucide-react";
import styles from "./ThinkingSection.module.css";

interface Props {
  content: string;
  isStreaming: boolean;
}

export function ThinkingSection({ content, isStreaming }: Props) {
  const [userOverride, setUserOverride] = useState<boolean | null>(null);
  const autoCollapseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const wasStreamingRef = useRef(false);
  const open = userOverride ?? isStreaming;
  const Chevron = open ? ChevronDown : ChevronRight;

  if (isStreaming) wasStreamingRef.current = true;

  useEffect(() => {
    if (!isStreaming && wasStreamingRef.current && userOverride === null) {
      autoCollapseTimer.current = setTimeout(() => {
        setUserOverride(false);
      }, 1500);
    }
    return () => {
      if (autoCollapseTimer.current) clearTimeout(autoCollapseTimer.current);
    };
  }, [isStreaming, userOverride]);

  useEffect(() => {
    if (isStreaming && userOverride === null) {
      setUserOverride(true);
    }
  }, [isStreaming, userOverride]);

  return (
    <div className={styles.section}>
      <button
        type="button"
        className={styles.header}
        onClick={() => setUserOverride(!open)}
        aria-expanded={open ? "true" : "false"}
      >
        <Chevron className={styles.chevron} />
        <span className={styles.label}>思考过程</span>
        {isStreaming && <span className={styles.dot} />}
      </button>
      {open && (
        <div className={styles.body}>
          <ReactMarkdown remarkPlugins={[remarkGfm]}>
            {content}
          </ReactMarkdown>
        </div>
      )}
    </div>
  );
}
