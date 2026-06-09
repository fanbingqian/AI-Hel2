import { useState } from "react";
import { useAuthStore } from "../../stores/authStore";
import { useSettingsStore } from "../../stores/settingsStore";
import { registerUser } from "../../services/api";
import styles from "./AuthForms.module.css";

interface Props {
  onSwitchToLogin: () => void;
}

export function RegisterForm({ onSwitchToLogin }: Props) {
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const setUser = useAuthStore((s) => s.setUser);
  const proceedAfterRegister = useAuthStore((s) => s.proceedAfterRegister);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");

    if (!username.trim()) {
      setError("请输入用户名");
      return;
    }
    if (password.length < 6) {
      setError("密码至少需要6位");
      return;
    }
    if (password !== confirmPassword) {
      setError("两次输入的密码不一致");
      return;
    }

    setLoading(true);
    try {
      const user = await registerUser(username.trim(), email.trim(), password) as any;
      setUser(user);
      useSettingsStore.getState().setUser(user);
      proceedAfterRegister();
    } catch (err: any) {
      setError(typeof err === "string" ? err : "注册失败，请稍后重试");
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
              placeholder="设置用户名"
              autoFocus
            />
          </div>
          <div className={styles.field}>
            <label>邮箱</label>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="your@email.com"
            />
          </div>
          <div className={styles.field}>
            <label>密码</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="至少6位密码"
            />
          </div>
          <div className={styles.field}>
            <label>确认密码</label>
            <input
              type="password"
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
              placeholder="再次输入密码"
            />
          </div>
          {error && <div className={styles.error}>{error}</div>}
          <button type="submit" className={styles.submitBtn} disabled={loading}>
            {loading ? "注册中..." : "注 册"}
          </button>
        </form>
        <div className={styles.switchLink}>
          已有账号？<a onClick={onSwitchToLogin}>立即登录</a>
        </div>
        <div className={styles.hint}>本地运行 · 数据安全</div>
      </div>
    </div>
  );
}
