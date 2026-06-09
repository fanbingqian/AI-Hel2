import { useState } from "react";
import { useAuthStore } from "../../stores/authStore";
import { useSettingsStore } from "../../stores/settingsStore";
import { loginUser } from "../../services/api";
import styles from "./AuthForms.module.css";

interface Props {
  onSwitchToRegister: () => void;
}

export function LoginForm({ onSwitchToRegister }: Props) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const setUser = useAuthStore((s) => s.setUser);
  const proceedAfterLogin = useAuthStore((s) => s.proceedAfterLogin);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!username.trim() || !password.trim()) {
      setError("请输入用户名和密码");
      return;
    }
    setLoading(true);
    setError("");
    try {
      const user = await loginUser(username.trim(), password) as any;
      setUser(user);
      useSettingsStore.getState().setUser(user);
      await proceedAfterLogin();
    } catch (err: any) {
      setError(typeof err === "string" ? err : "登录失败，请检查用户名和密码");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className={styles.overlay}>
      <div className={styles.card}>
        <div className={styles.logo}>AI-<span>Hel2</span></div>
        <form onSubmit={handleSubmit}>
          <div className={styles.field}>
            <label>用户名</label>
            <input
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="输入用户名"
              autoFocus
            />
          </div>
          <div className={styles.field}>
            <label>密码</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="输入密码"
            />
          </div>
          {error && <div className={styles.error}>{error}</div>}
          <button type="submit" className={styles.submitBtn} disabled={loading}>
            {loading ? "登录中..." : "登 录"}
          </button>
        </form>
        <div className={styles.switchLink}>
          没有账号？<a onClick={onSwitchToRegister}>立即注册</a>
        </div>
        <div className={styles.hint}>本地运行 · 数据安全</div>
      </div>
    </div>
  );
}
