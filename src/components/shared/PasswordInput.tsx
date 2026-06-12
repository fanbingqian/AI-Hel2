import { useState } from "react";

interface Props {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  id?: string;
  name?: string;
  autoComplete?: string;
  autoFocus?: boolean;
  className?: string;
  style?: React.CSSProperties;
  onKeyDown?: (e: React.KeyboardEvent) => void;
  onFocus?: (e: React.FocusEvent) => void;
  ariaLabel?: string;
}

/** Password field with a visible show/hide toggle button. */
export function PasswordInput({
  value, onChange, placeholder, id, name, autoComplete, autoFocus,
  className, style, onKeyDown, onFocus, ariaLabel,
}: Props) {
  const [show, setShow] = useState(false);

  return (
    <div style={{ display: "flex", gap: 4, alignItems: "center", flex: 1, minWidth: 0 }}>
      <input
        id={id}
        name={name}
        type={show ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        autoComplete={autoComplete}
        autoFocus={autoFocus}
        className={className}
        style={{ flex: 1, minWidth: 0, ...style }}
        onKeyDown={onKeyDown}
        onFocus={onFocus}
        aria-label={ariaLabel}
      />
      <button
        type="button"
        onClick={() => setShow(!show)}
        title={show ? "隐藏" : "显示"}
        style={{
          background: "transparent",
          border: "1px solid #555",
          borderRadius: 4,
          color: "#b3b3b3",
          cursor: "pointer",
          padding: "4px 7px",
          fontSize: 11,
          flexShrink: 0,
          fontFamily: "inherit",
          lineHeight: 1.2,
        }}
      >
        {show ? "隐藏" : "显示"}
      </button>
    </div>
  );
}
