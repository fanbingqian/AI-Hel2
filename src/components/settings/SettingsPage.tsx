import { useState, useEffect, useCallback } from "react";
import { useSettingsStore } from "../../stores/settingsStore";
import { useAuthStore } from "../../stores/authStore";
import { AgentSettings } from "./AgentSettings";
import { UpdateDialog } from "./UpdateDialog";
import { PasswordInput } from "../shared/PasswordInput";
import { invoke } from "@tauri-apps/api/core";
import {
  changePassword,
  saveUserProfile,
  saveConfig,
  exportData,
  importData,
  getAgentStatus,
  restartAgent,
  getAvatar,
  saveAvatar,
  readFileBase64,
  listPlatformStatus,
  gatewayQrStart,
  gatewayQrPoll,
  gatewayQrCancel,
  gatewaySaveCredentials,
  gatewaySavePlatformConfig,
  gatewayRemovePlatform,
  listCronJobs,
  addCronJob,
  updateCronJob,
  deleteCronJob,
  toggleCronJob,
  triggerCronJob,
  getCronOutput,
  type CronJob,
  type PlatformConfigStatus,
  type QrSessionInfo,
} from "../../services/api";
import styles from "./SettingsPage.module.css";

type SettingsSection =
  | "account"
  | "gateway"
  | "theme"
  | "language"
  | "voice"
  | "update"
  | "migration"
  | "tasks"
  | "nexus"
  | "agents";

const navItems: { id: SettingsSection; label: string }[] = [
  { id: "account", label: "账户信息" },
  { id: "agents", label: "大模型" },
  { id: "gateway", label: "网关配置" },
  { id: "theme", label: "主题" },
  { id: "language", label: "语言" },
  { id: "voice", label: "语音" },
  { id: "update", label: "更新" },
  { id: "migration", label: "数据迁移" },
  { id: "tasks", label: "定时任务" },
  { id: "nexus", label: "知识引擎" },
];

function AccountSection() {
  const user = useSettingsStore((s) => s.user);
  const setUser = useSettingsStore((s) => s.setUser);
  const isLoggedIn = useSettingsStore((s) => s.isLoggedIn);
  const resetAuth = useAuthStore((s) => s.resetAuth);
  const [passwordModal, setPasswordModal] = useState(false);
  const [oldPw, setOldPw] = useState("");
  const [newPw, setNewPw] = useState("");
  const [pwError, setPwError] = useState("");
  const [pwSuccess, setPwSuccess] = useState("");
  const [saving, setSaving] = useState(false);

  // Password visibility toggles
  const [showOldPw, setShowOldPw] = useState(false);
  const [showNewPw, setShowNewPw] = useState(false);

  // Avatar
  const [avatarUrl, setAvatarUrl] = useState<string | null>(null);

  useEffect(() => {
    getAvatar().then((url) => { if (url) setAvatarUrl(url); }).catch(() => {});
  }, []);

  const handleAvatarClick = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: false,
        filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp"] }],
      });
      if (!selected) return;
      const path = selected as string;
      const b64 = await readFileBase64(path);
      await saveAvatar(b64);
      setAvatarUrl(b64);
    } catch (e) {
      console.error("Avatar upload failed:", e);
    }
  };

  const handleSaveProfile = useCallback(async () => {
    if (!user) return;
    setSaving(true);
    try {
      await saveUserProfile(user.name, user.email || "");
    } catch { /* ignore */ }
    setSaving(false);
  }, [user]);

  const handleChangePassword = async () => {
    setPwError("");
    setPwSuccess("");
    if (!user || !oldPw || !newPw) {
      setPwError("请填写所有字段");
      return;
    }
    if (newPw.length < 6) {
      setPwError("新密码长度不能少于6位");
      return;
    }
    try {
      await changePassword(user.name, oldPw, newPw);
      setPwSuccess("密码修改成功");
      setOldPw("");
      setNewPw("");
    } catch (e: any) {
      setPwError(String(e));
    }
  };

  const handleLogout = () => {
    resetAuth();
  };

  if (!isLoggedIn || !user) {
    return (
      <div className={styles.section}>
        <h2 className={styles.sectionTitle}>账户信息</h2>
        <div className={styles.emptyHint}>未登录</div>
      </div>
    );
  }

  const initial = user.name?.charAt(0) || "?";

  return (
    <div className={styles.section}>
      <h2 className={styles.sectionTitle}>账户信息</h2>
      <div className={styles.accountLayout}>
        <div className={styles.accountAvatarCol}>
          <div
            className={styles.avatar}
            title="点击更换头像"
            onClick={handleAvatarClick}
            style={{ cursor: "pointer" }}
          >
            {avatarUrl ? (
              <img src={avatarUrl} alt="头像" style={{ width: "100%", height: "100%", objectFit: "cover" }} />
            ) : (
              <span>{initial}</span>
            )}
            <div className={styles.avatarOverlay}>更换</div>
          </div>
          <div className={styles.caption}>点击更换头像</div>
        </div>
        <div className={styles.accountFields}>
          <div className={styles.fieldGroup}>
            <label className={styles.label}>用户名</label>
            <input
              className={styles.textInput}
              value={user.name}
              onChange={(e) => setUser({ ...user, name: e.target.value })}
              onBlur={handleSaveProfile}
            />
          </div>
          <div className={styles.fieldGroup}>
            <label className={styles.label}>邮箱</label>
            <input
              className={styles.textInput}
              value={user.email || ""}
              onChange={(e) => setUser({ ...user, email: e.target.value })}
              onBlur={handleSaveProfile}
              placeholder="zhangsan@example.com"
            />
          </div>
        </div>
      </div>
      <div className={styles.fieldGroup}>
        <label className={styles.label}>密码</label>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <div style={{ display: "flex", alignItems: "center", maxWidth: 200, flex: 1 }}>
            <input
              className={styles.textInput}
              type="password"
              value="********"
              readOnly
              style={{ maxWidth: 200 }}
            />
          </div>
          <button className={styles.btnPrimary} onClick={() => setPasswordModal(!passwordModal)}>
            {passwordModal ? "取消" : "修改密码"}
          </button>
        </div>
        {passwordModal && (
          <div style={{ marginTop: 10, padding: 12, background: "#2C2C2C", borderRadius: 6, maxWidth: 360 }}>
            <div className={styles.fieldGroup}>
              <label className={styles.label}>原密码</label>
              <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
                <input
                  className={styles.textInput}
                  type={showOldPw ? "text" : "password"}
                  value={oldPw}
                  onChange={(e) => setOldPw(e.target.value)}
                />
                <button
                  type="button"
                  onClick={() => setShowOldPw(!showOldPw)}
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
                  title={showOldPw ? "隐藏密码" : "显示密码"}
                >
                  {showOldPw ? "隐藏" : "显示"}
                </button>
              </div>
            </div>
            <div className={styles.fieldGroup}>
              <label className={styles.label}>新密码</label>
              <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
                <input
                  className={styles.textInput}
                  type={showNewPw ? "text" : "password"}
                  value={newPw}
                  onChange={(e) => setNewPw(e.target.value)}
                />
                <button
                  type="button"
                  onClick={() => setShowNewPw(!showNewPw)}
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
                  title={showNewPw ? "隐藏密码" : "显示密码"}
                >
                  {showNewPw ? "隐藏" : "显示"}
                </button>
              </div>
            </div>
            {pwError && <div style={{ color: "#fa5151", fontSize: 12, marginBottom: 8 }}>{pwError}</div>}
            {pwSuccess && <div style={{ color: "#07c160", fontSize: 12, marginBottom: 8 }}>{pwSuccess}</div>}
            <button className={styles.btnPrimary} onClick={handleChangePassword}>确认修改</button>
          </div>
        )}
      </div>
      <div className={styles.divider} />
      <div style={{ display: "flex", gap: 8 }}>
        {saving && <span style={{ fontSize: 11, color: "#07c160" }}>已保存</span>}
      </div>
      <button className={styles.btnDanger} onClick={handleLogout}>退出登录</button>
    </div>
  );
}

function GatewaySection() {
  const [platforms, setPlatforms] = useState<PlatformConfigStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [agentRunning, setAgentRunning] = useState(false);
  const [restarting, setRestarting] = useState(false);

  // QR flow state
  const [qrPlatform, setQrPlatform] = useState<string | null>(null);
  const [qrSession, setQrSession] = useState<QrSessionInfo | null>(null);
  const [qrStatus, setQrStatus] = useState<string>("");
  const [qrMessage, setQrMessage] = useState<string>("");
  const [qrPolling, setQrPolling] = useState(false);
  const [qrTimer, setQrTimer] = useState<ReturnType<typeof setInterval> | null>(null);

  const loadPlatforms = useCallback(async () => {
    try {
      const status = await listPlatformStatus();
      setPlatforms(status);
    } catch { /* ignore */ }
    setLoading(false);
  }, []);

  useEffect(() => {
    loadPlatforms();
    // Check agent status immediately, then poll every 10s
    const checkStatus = () => {
      getAgentStatus().then((s: any) => setAgentRunning(s?.healthy ?? false)).catch(() => {});
    };
    checkStatus();
    const interval = setInterval(checkStatus, 10000);
    return () => clearInterval(interval);
  }, [loadPlatforms]);

  const handleRestart = async () => {
    setRestarting(true);
    try {
      await restartAgent();
      setTimeout(() => {
        getAgentStatus().then((s: any) => setAgentRunning(s?.healthy ?? false)).catch(() => {});
        setRestarting(false);
      }, 3000);
    } catch { setRestarting(false); }
  };

  const handleTogglePlatform = async (p: PlatformConfigStatus) => {
    const newEnabled = !p.enabled;
    const config = newEnabled
      ? { enabled: true }
      : { enabled: false };
    try {
      await gatewaySavePlatformConfig(p.key, config);
      setPlatforms(prev => prev.map(x => x.key === p.key ? { ...x, enabled: newEnabled } : x));
    } catch (e: any) { alert(`操作失败: ${e}`); }
  };

  // ── QR Flow ──
  const startQrFlow = async (platformKey: string) => {
    setQrPlatform(platformKey);
    setQrStatus("starting");
    setQrMessage("");
    try {
      const session = await gatewayQrStart(platformKey);
      setQrSession(session);
      setQrStatus("waiting_scan");
      // Start polling
      startPolling(platformKey);
    } catch (e: any) {
      setQrStatus("error");
      setQrMessage(`启动失败: ${e}`);
    }
  };

  const startPolling = (platformKey: string) => {
    setQrPolling(true);
    const timer = setInterval(async () => {
      try {
        const result = await gatewayQrPoll(platformKey);
        if (result.status === "success") {
          clearInterval(timer);
          setQrTimer(null);
          setQrPolling(false);
          setQrStatus("success");
          setQrMessage("注册成功！");
          if (result.credentials) {
            await gatewaySaveCredentials(platformKey, result.credentials);
          }
          loadPlatforms();
        } else if (result.status === "failed" || result.status === "expired") {
          clearInterval(timer);
          setQrTimer(null);
          setQrPolling(false);
          setQrStatus(result.status);
          setQrMessage(result.message || (result.status === "expired" ? "二维码已过期" : "授权失败"));
        } else if (result.status === "scanned") {
          setQrStatus("scanned");
          setQrMessage(result.message || "已扫码，请在手机上确认...");
        } else if (result.status === "refreshed" && result.qr_url) {
          setQrSession(prev => prev ? { ...prev, qr_url: result.qr_url! } : null);
        }
      } catch {
        // polling errors are expected
      }
    }, 3000);
    setQrTimer(timer);
  };

  const cancelQrFlow = async () => {
    if (qrTimer) { clearInterval(qrTimer); setQrTimer(null); }
    setQrPolling(false);
    if (qrPlatform) {
      try { await gatewayQrCancel(qrPlatform); } catch { /* ignore */ }
    }
    setQrPlatform(null);
    setQrSession(null);
    setQrStatus("");
    setQrMessage("");
  };

  const handleRemovePlatform = async (p: PlatformConfigStatus) => {
    if (!confirm(`确定要移除 ${p.label} 的配置吗？`)) return;
    try {
      await gatewayRemovePlatform(p.key);
      setPlatforms(prev => prev.map(x => x.key === p.key ? { ...x, configured: false, enabled: false } : x));
    } catch (e: any) { alert(`移除失败: ${e}`); }
  };

  if (loading) return <div className={styles.section}><div className={styles.desc}>加载中...</div></div>;

  const qrPlatforms = platforms.filter(p => p.has_qr);
  const otherPlatforms = platforms.filter(p => !p.has_qr);

  return (
    <div className={styles.section}>
      <h2 className={styles.sectionTitle}>网关配置</h2>
      <div className={styles.desc} style={{ marginBottom: 16 }}>
        管理消息平台的连接与配置。二维码注册平台支持扫码一键接入，其他平台需手动填写环境变量。
      </div>

      <div className={styles.gatewayBanner}>
        <div>
          <div className={styles.gatewayBannerInfo}>消息网关</div>
          <div className={styles.gatewayBannerSub}>控制所有消息平台的连接状态</div>
        </div>
        <div className={styles.gatewayBannerRight}>
          <span className={styles.gatewayStatus}>
            <span className={agentRunning ? styles.svcDotOn : styles.svcDotOff} />
            <span style={{ color: agentRunning ? "#07c160" : "#fa5151" }}>
              {agentRunning ? "运行中" : "已停止"}
            </span>
          </span>
          <button
            className={agentRunning ? styles.btnSmDanger : styles.btnSmPrimary}
            style={{ fontSize: 11, padding: "5px 14px" }}
            onClick={handleRestart}
            disabled={restarting}
          >
            {restarting ? "重启中..." : agentRunning ? "停止网关" : "启动网关"}
          </button>
        </div>
      </div>

      {/* QR-supported platforms */}
      {qrPlatforms.length > 0 && (
        <>
          <div className={styles.label} style={{ marginBottom: 10 }}>二维码注册平台 ({qrPlatforms.length})</div>
          <div className={styles.pltGrid}>
            {qrPlatforms.map((p) => (
              <div key={p.key} className={styles.pltCard}>
                <div>
                  <div className={styles.pltName}>
                    {p.label}
                    {p.configured && <span style={{ color: "#07c160", marginLeft: 6, fontSize: 11 }}>已配置</span>}
                  </div>
                  <div className={styles.pltDesc}>{p.key}</div>
                </div>
                <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                  {p.configured ? (
                    <>
                      <span
                        className={`${styles.toggleSwitch} ${p.enabled ? styles.tgOn : styles.tgOff}`}
                        onClick={() => handleTogglePlatform(p)}
                      >
                        <span className={styles.tgKnob} style={{ transform: p.enabled ? "translateX(18px)" : "translateX(2px)" }} />
                      </span>
                      <button className={styles.btnSmDanger} style={{ fontSize: 10, padding: "3px 8px" }} onClick={() => handleRemovePlatform(p)}>
                        移除
                      </button>
                    </>
                  ) : (
                    <button
                      className={styles.btnSmPrimary}
                      style={{ fontSize: 11, padding: "4px 12px" }}
                      onClick={() => startQrFlow(p.key)}
                    >
                      扫码注册
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </>
      )}

      {/* QR Modal */}
      {qrPlatform && qrSession && (
        <div style={{
          position: "fixed", top: 0, left: 0, right: 0, bottom: 0,
          background: "rgba(0,0,0,0.5)", display: "flex", alignItems: "center", justifyContent: "center",
          zIndex: 1000,
        }} onClick={cancelQrFlow}>
          <div style={{
            background: "var(--bg-primary, #1e1e1e)", borderRadius: 12, padding: 24,
            minWidth: 360, maxWidth: 440,
            border: "1px solid var(--border-color, #333)",
          }} onClick={e => e.stopPropagation()}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
              <h3 style={{ margin: 0, color: "var(--text-primary, #e0e0e0)" }}>
                注册 {qrPlatforms.find(p => p.key === qrPlatform)?.label || qrPlatform}
              </h3>
              <button onClick={cancelQrFlow} style={{
                background: "none", border: "none", color: "var(--text-secondary, #999)",
                fontSize: 20, cursor: "pointer",
              }}>×</button>
            </div>

            {qrStatus === "error" ? (
              <div style={{ color: "#fa5151", marginBottom: 12 }}>{qrMessage}</div>
            ) : qrStatus === "success" ? (
              <div style={{ color: "#07c160", marginBottom: 12, fontSize: 16, textAlign: "center", padding: 20 }}>
                {qrMessage}
              </div>
            ) : (
              <>
                <div style={{ textAlign: "center", marginBottom: 12 }}>
                  <img src={`https://api.qrserver.com/v1/create-qr-code/?size=200x200&data=${encodeURIComponent(qrSession.qr_url)}`}
                    alt="QR Code" style={{ width: 200, height: 200, background: "#fff", borderRadius: 8 }} />
                </div>
                <div style={{ fontSize: 12, color: "var(--text-secondary, #999)", textAlign: "center", marginBottom: 12 }}>
                  {qrStatus === "scanned" ? qrMessage : "请使用对应平台 App 扫描二维码"}
                </div>
                {qrPolling && qrStatus !== "scanned" && (
                  <div style={{ textAlign: "center", color: "var(--text-secondary, #999)", fontSize: 12 }}>
                    等待扫码中{qrStatus === "waiting_scan" ? "..." : ""}
                  </div>
                )}
                {qrStatus === "expired" && (
                  <div style={{ textAlign: "center", marginTop: 8 }}>
                    <button className={styles.btnSmPrimary} onClick={() => startQrFlow(qrPlatform)}>
                      重新获取二维码
                    </button>
                  </div>
                )}
                <div style={{ textAlign: "center", marginTop: 8 }}>
                  <button className={styles.btnSmDanger} style={{ fontSize: 11 }} onClick={cancelQrFlow}>
                    取消
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {/* Other platforms (no QR support) */}
      {otherPlatforms.length > 0 && (
        <>
          <div className={styles.label} style={{ marginBottom: 10, marginTop: 20 }}>
            其他平台 ({otherPlatforms.length})
          </div>
          <div className={styles.desc} style={{ marginBottom: 10 }}>
            以下平台需要在 ~/.hermes/.env 中手动配置 API Key/Token 等环境变量
          </div>
          <div className={styles.pltGrid}>
            {otherPlatforms.map((p) => (
              <div key={p.key} className={styles.pltCard}>
                <div>
                  <div className={styles.pltName}>
                    {p.label}
                    {p.configured && <span style={{ color: "#07c160", marginLeft: 6, fontSize: 11 }}>已配置</span>}
                  </div>
                  <div className={styles.pltDesc}>{p.key}</div>
                </div>
                <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                  {p.configured && (
                    <>
                      <span
                        className={`${styles.toggleSwitch} ${p.enabled ? styles.tgOn : styles.tgOff}`}
                        onClick={() => handleTogglePlatform(p)}
                      >
                        <span className={styles.tgKnob} style={{ transform: p.enabled ? "translateX(18px)" : "translateX(2px)" }} />
                      </span>
                      <button className={styles.btnSmDanger} style={{ fontSize: 10, padding: "3px 8px" }} onClick={() => handleRemovePlatform(p)}>
                        移除
                      </button>
                    </>
                  )}
                  {!p.configured && (
                    <span style={{ fontSize: 11, color: "var(--text-secondary, #999)" }}>手动配置</span>
                  )}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function ThemeSection() {
  const theme = useSettingsStore((s) => s.theme);

  const applyTheme = (t: "dark" | "light") => {
    document.documentElement.setAttribute("data-theme", t);
  };

  const handleThemeChange = async (t: "dark" | "light" | "system") => {
    useSettingsStore.setState({ theme: t });
    try {
      await saveConfig({ appearance: { theme: t } });
    } catch { /* ignore */ }
    if (t === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      applyTheme(mq.matches ? "dark" : "light");
    } else {
      applyTheme(t);
    }
  };

  return (
    <div className={styles.section}>
      <h2 className={styles.sectionTitle}>主题</h2>
      <label className={styles.radioRow} onClick={() => handleThemeChange("system")}>
        <input type="radio" name="theme" checked={theme === "system"} readOnly />
        跟随系统 <span className={styles.radioHint}>自动切换暗色/亮色</span>
      </label>
      <label className={styles.radioRow} onClick={() => handleThemeChange("dark")}>
        <input type="radio" name="theme" checked={theme === "dark"} readOnly />
        暗色 <span className={styles.radioHint}>暗灰底色 + 微信绿强调</span>
      </label>
      <label className={styles.radioRow} onClick={() => handleThemeChange("light")}>
        <input type="radio" name="theme" checked={theme === "light"} readOnly />
        亮色 <span className={styles.radioHint}>浅色背景 + 绿色强调</span>
      </label>
    </div>
  );
}

function LanguageSection() {
  const language = useSettingsStore((s) => s.language);

  const handleLanguageChange = async (lang: string) => {
    const langCode: "zh-CN" | "en" = lang === "en" ? "en" : "zh-CN";
    useSettingsStore.setState({ language: langCode });
    try {
      await saveConfig({ language: langCode });
    } catch { /* ignore */ }
  };

  return (
    <div className={styles.section}>
      <h2 className={styles.sectionTitle}>语言</h2>
      <select
        className={styles.select}
        defaultValue={language === "zh-CN" ? "zh" : "en"}
        onChange={(e) => handleLanguageChange(e.target.value === "en" ? "en" : "zh")}
      >
        <option value="zh">简体中文</option>
        <option value="en">English</option>
        <option value="ja">日本語</option>
      </select>
    </div>
  );
}

const SPEAKERS = [
  { id: 0, name: "苏映雪", gender: "女" },
  { id: 1, name: "顾年", gender: "男" },
  { id: 2, name: "傅诗雨", gender: "女" },
  { id: 3, name: "病娇", gender: "女" },
  { id: 4, name: "霸总", gender: "男" },
];

function VoiceSection() {
  const ttsSpeaker = useSettingsStore((s) => s.ttsSpeaker);
  const ttsEnabled = useSettingsStore((s) => s.ttsEnabled);
  const setTtsSpeaker = useSettingsStore((s) => s.setTtsSpeaker);
  const setTtsEnabled = useSettingsStore((s) => s.setTtsEnabled);
  const [previewing, setPreviewing] = useState<number | null>(null);

  const handlePreview = async (speakerId: number) => {
    setPreviewing(speakerId);
    try {
      const base64: string = await invoke("tts_preview", { speaker: speakerId });
      const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
      const blob = new Blob([bytes], { type: "audio/wav" });
      const url = URL.createObjectURL(blob);
      const audio = new Audio(url);
      audio.onended = () => URL.revokeObjectURL(url);
      audio.onerror = () => URL.revokeObjectURL(url);
      await audio.play();
    } catch (e) {
      console.error("Preview failed:", e);
    }
    setPreviewing(null);
  };

  return (
    <div className={styles.section}>
      <h2 className={styles.sectionTitle}>语音</h2>

      <div className={styles.fieldGroup}>
        <label className={styles.toggle} style={{ cursor: "pointer" }} onClick={() => setTtsEnabled(!ttsEnabled)}>
          <span className={styles.toggleLabel}>自动语音播报 (TTS)</span>
          <span className={`${styles.toggleSwitch} ${ttsEnabled ? styles.tgOn : styles.tgOff}`}>
            <span className={styles.tgKnob} style={{ transform: ttsEnabled ? "translateX(18px)" : "translateX(2px)" }} />
          </span>
        </label>
        <div className={styles.desc}>{ttsEnabled ? "回复完成后自动朗读" : "回复仅文字显示"}</div>
      </div>

      <div className={styles.dividerSm}>
        <div className={styles.label} style={{ marginBottom: 10 }}>音色选择</div>
        <div className={styles.desc} style={{ marginBottom: 10 }}>
          选择你喜欢的语音音色，点击试听按钮预览效果。
        </div>
        {SPEAKERS.map((s) => (
          <div key={s.id} className={styles.card} style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "10px 14px",
            marginBottom: 6,
            background: ttsSpeaker === s.id ? "rgba(7,193,96,0.08)" : undefined,
            border: ttsSpeaker === s.id ? "1px solid rgba(7,193,96,0.25)" : undefined,
          }}>
            <div>
              <div style={{ fontSize: 13, color: "#e6e6e6", fontWeight: 500 }}>
                {s.name}
                {ttsSpeaker === s.id && <span style={{ color: "#07c160", marginLeft: 8, fontSize: 11 }}>当前</span>}
              </div>
              <div className={styles.desc}>{s.gender}声</div>
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <button
                className={styles.btnSmPrimary}
                style={{ fontSize: 11 }}
                onClick={() => handlePreview(s.id)}
                disabled={previewing === s.id}
              >
                {previewing === s.id ? "合成中..." : "试听"}
              </button>
              {ttsSpeaker !== s.id && (
                <button
                  className={styles.btnSmPrimary}
                  style={{ fontSize: 11, background: "#3F3F3F" }}
                  onClick={() => setTtsSpeaker(s.id)}
                >
                  选择
                </button>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function UpdateSection() {
  const [showUpdate, setShowUpdate] = useState(false);

  return (
    <div className={styles.section}>
      <h2 className={styles.sectionTitle}>更新</h2>
      <div className={styles.fieldGroup}>
        <button className={styles.btnPrimary} onClick={() => setShowUpdate(true)}>检查更新</button>
      </div>
      {showUpdate && <UpdateDialog onClose={() => setShowUpdate(false)} />}
    </div>
  );
}

function MigrationSection() {
  const [exporting, setExporting] = useState(false);
  const [exportResult, setExportResult] = useState("");
  const [importing, setImporting] = useState(false);

  const handleExport = async () => {
    setExporting(true);
    try {
      const path = await exportData() as string;
      setExportResult(`导出成功: ${path}`);
    } catch (e: any) {
      setExportResult(`导出失败: ${e}`);
    }
    setExporting(false);
  };

  const handleImport = async () => {
    setImporting(true);
    try {
      // In a real scenario this would open a file dialog
      setExportResult("请通过文件对话框选择 ZIP 文件导入");
    } catch (e: any) {
      setExportResult(`导入失败: ${e}`);
    }
    setImporting(false);
  };

  return (
    <div className={styles.section}>
      <h2 className={styles.sectionTitle}>数据迁移</h2>
      <div className={styles.desc} style={{ marginBottom: 16 }}>
        导出或导入知识库、对话历史、配置等数据。
      </div>
      <div style={{ display: "flex", gap: 10, marginBottom: 16 }}>
        <button className={styles.btnPrimary} onClick={handleExport} disabled={exporting}>
          {exporting ? "导出中..." : "导出数据"}
        </button>
        <button className={styles.btnPrimary} onClick={handleImport} disabled={importing}>
          {importing ? "导入中..." : "导入数据"}
        </button>
      </div>
      <div className={styles.desc}>支持格式: ZIP 归档（含 SQLite + Markdown + 配置文件）</div>
      {exportResult && (
        <div className={styles.alert} style={{ color: exportResult.includes("失败") ? "#fa5151" : "#07c160" }}>
          {exportResult}
        </div>
      )}
    </div>
  );
}

function NexusSection() {
  const [nexusConfig, setNexusConfig] = useState<any>(null);
  const [testResult, setTestResult] = useState<any>(null);
  const [saving, setSaving] = useState(false);

  // Maintenance state
  const [maintStatus, setMaintStatus] = useState<any>(null);
  const [runningTask, setRunningTask] = useState<string | null>(null);
  const [taskResult, setTaskResult] = useState<any>(null);

  // Server health state
  const [serverHealth, setServerHealth] = useState<{ running: boolean; port: number; url: string } | null>(null);
  const [checkingHealth, setCheckingHealth] = useState(false);

  useEffect(() => {
    invoke("get_nexus_config").then(setNexusConfig).catch(() => {});
    refreshMaintenanceStatus();
    refreshServerHealth();
  }, []);

  const refreshMaintenanceStatus = async () => {
    try {
      const s = await invoke("nexus_get_maintenance_status");
      setMaintStatus(s);
    } catch { /* ignore */ }
  };

  const refreshServerHealth = async () => {
    setCheckingHealth(true);
    try {
      const h = await invoke("check_nexus_server_health");
      setServerHealth(h as any);
    } catch {
      setServerHealth(null);
    }
    setCheckingHealth(false);
  };

  const handleModeChange = async (mode: string) => {
    const updated = { ...nexusConfig, llm_mode: mode };
    setNexusConfig(updated);
    try {
      setSaving(true);
      await invoke("save_nexus_config", { config: updated });
    } catch (e) {
      console.error("Save nexus config failed:", e);
    }
    setSaving(false);
  };

  const handleFieldChange = async (field: string, value: string) => {
    if (!nexusConfig) return;
    const updated = { ...nexusConfig, [field]: value };
    setNexusConfig(updated);
    // Auto-save on every change so config persists across page switches
    try { await invoke("save_nexus_config", { config: updated }); } catch {}
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await invoke("save_nexus_config", { config: nexusConfig });
    } catch (e) {
      console.error("Save nexus config failed:", e);
    }
    setSaving(false);
  };

  const runMaintenance = async (task: string, cmd: string, args?: Record<string, unknown>) => {
    setRunningTask(task);
    setTaskResult(null);
    try {
      // Group A: Health check = quality + fix_migrated (chained)
      if (task === "health_check") {
        const qResult = await invoke("nexus_maintain_quality");
        const fResult = await invoke("nexus_maintain_fix_migrated");
        setTaskResult({
          task: "health_check",
          status: "completed",
          summary: `质量评分: ${(qResult as any).summary || "完成"} | 修复迁移: ${(fResult as any).summary || "完成"}`,
          details: [
            ...((qResult as any)?.details || []).map((d: any) => ({ ...d, source: "quality" })),
            ...((fResult as any)?.details || []).map((d: any) => ({ ...d, source: "fix_migrated" })),
          ],
        });
      } else {
        const result = await invoke(cmd, args ?? {});
        setTaskResult(result);
      }
      await refreshMaintenanceStatus();
    } catch (e: any) {
      setTaskResult({ error: String(e) });
    }
    setRunningTask(null);
  };

  const maintProps = { runningTask, runMaintenance };

  if (!nexusConfig) return <div className={styles.section}><h2 className={styles.sectionTitle}>知识引擎</h2><div className={styles.emptyHint}>加载中...</div></div>;

  const llmMode = nexusConfig.llm_mode || "custom";
  const status = maintStatus;

  return (
    <div className={styles.section}>
      <h2 className={styles.sectionTitle}>知识引擎 (Nexus)</h2>
      <div className={styles.desc} style={{ marginBottom: 12 }}>
        Nexus 知识引擎负责将对话、文档自动提取为知识图谱，并维护知识库质量。
      </div>

      {/* ── Server Health Indicator ── */}
      <div style={{
        display: "flex", alignItems: "center", gap: 10, marginBottom: 14,
        padding: "10px 12px", borderRadius: 6,
        background: "#2C2C2C",
        border: "1px solid #333333",
      }}>
        <span style={{
          width: 10, height: 10, borderRadius: "50%",
          background: serverHealth ? "#4caf50" : "#f44336",
          display: "inline-block",
          boxShadow: `0 0 6px ${serverHealth ? "#4caf50" : "#f44336"}`,
        }} />
        <span style={{ fontSize: 13, color: "#e6e6e6" }}>
          {checkingHealth ? "检测中..." : serverHealth ? `服务运行中 · 端口 ${serverHealth.port}` : "服务未启动"}
        </span>
        <button
          type="button"
          onClick={refreshServerHealth}
          style={{ marginLeft: "auto", padding: "3px 10px", fontSize: 11, cursor: "pointer", borderRadius: 4, border: "1px solid #555", background: "transparent", color: "#aaa" }}
        >
          重新检测
        </button>
      </div>

      {/* ── Maintenance Status ── */}
      <div className={styles.dividerSm}>
        <div className={styles.label} style={{ marginBottom: 10 }}>知识库状态</div>
        {status ? (
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "6px 16px", marginBottom: 12 }}>
            <StatusItem label="实体总数" value={status.total_entities} />
            <StatusItem label="上次整理" value={status.last_maintenance ? status.last_maintenance.slice(0, 10) : "从未"} />
            <StatusItem label="低质量" value={status.low_quality_count} warn={status.low_quality_count > 0} />
            <StatusItem label="孤岛实体" value={status.orphan_count} warn={status.orphan_count > 0} />
            <StatusItem label="疑似重复" value={status.duplicate_candidates} warn={status.duplicate_candidates > 0} />
            <StatusItem label="待修复迁移" value={status.migration_needs_fix} warn={status.migration_needs_fix > 0} />
          </div>
        ) : (
          <div className={styles.desc} style={{ marginBottom: 12 }}>加载中...</div>
        )}
        <button
          className={styles.btnSmPrimary}
          style={{ fontSize: 11 }}
          onClick={refreshMaintenanceStatus}
        >
          刷新状态
        </button>
      </div>

      {/* ── Maintenance Actions ── */}
      <div className={styles.dividerSm}>
        <div className={styles.label} style={{ marginBottom: 4 }}>维护操作</div>
        <div className={styles.desc} style={{ marginBottom: 14 }}>
          A-C 涉及 LLM 调用会消耗 token。D/F 纯规则/算法，不消耗 token。
        </div>

        {/* ── Reset Graph (admin tool) ── */}
        <div style={{
          marginBottom: 14, padding: "10px 12px", borderRadius: 6,
          background: "rgba(250,81,81,0.06)", border: "1px solid rgba(250,81,81,0.15)",
        }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
            <div>
              <span style={{ fontSize: 12, color: "#fa5151", fontWeight: 500 }}>⚠ 重置知识图谱</span>
              <div style={{ fontSize: 11, color: "#808080", marginTop: 2 }}>
                删除所有旧实体和关系（保留文档节点），清除提取缓存。执行后需运行「全量归类」重新提取。
              </div>
            </div>
            <button
              type="button"
              onClick={async () => {
                if (!confirm("确定要删除所有旧实体和关系吗？\n\n文档节点（__file__）将被保留。\n执行后需要重新提取所有文档。")) return;
                setRunningTask("reset");
                setTaskResult(null);
                try {
                  const { nexusResetGraph } = await import("../../services/api");
                  const result = await nexusResetGraph();
                  setTaskResult({ task: "reset", summary: `已删除 ${(result as any).deleted_entities} 个实体、${(result as any).deleted_relations} 条关系。保留了 ${(result as any).kept_document_entities} 个文档节点。` });
                  await refreshMaintenanceStatus();
                } catch (e: any) { setTaskResult({ error: String(e) }); }
                setRunningTask(null);
              }}
              disabled={runningTask !== null}
              style={{
                padding: "4px 12px", fontSize: 11, cursor: "pointer", borderRadius: 4,
                border: "1px solid rgba(250,81,81,0.4)", background: "transparent", color: "#fa5151",
                opacity: runningTask ? 0.5 : 1,
              }}
            >
              {runningTask === "reset" ? "重置中..." : "重置图谱"}
            </button>
          </div>
        </div>

        <MaintGroup
          layer="A · 知识库健康检查"
          color="#1a7a3a"
          items={[
            {
              task: "health_check", cmd: "nexus_maintain_quality",
              label: "运行检查",
              desc: "实体分级评分（A/B/C/D）+ 类型引号清理 + 修复迁移数据格式。纯 SQL 规则，不消耗 token。",
            },
          ]}
          {...maintProps}
        />

        <MaintGroup
          layer="B · 实体去重与合并"
          color="#2a5a1a"
          items={[
            {
              task: "dedup", cmd: "nexus_maintain_dedup",
              label: "运行去重",
              desc: "Blocking 预分块 → LLM 批量判断重复（跨类型语义匹配）→ confidence≥0.95 自动合并，0.85-0.95 标记 SAME_AS 审核。需调用 LLM。",
              llm: true,
            },
            {
              task: "analyze_types", cmd: "nexus_analyze_types",
              label: "类型分析",
              desc: "扫描所有实体类型，发现语义相似的类型变体（如 'concept'→'概念'），生成标准化建议。纯统计，不消耗 token。",
            },
          ]}
          {...maintProps}
        />

        <MaintGroup
          layer="C · 文档归类"
          color="#1a5a3a"
          items={[
            {
              task: "classify", cmd: "nexus_maintain_classify", args: { fullScan: false },
              label: "增量归类",
              desc: "扫描 wiki 根目录未分类文件 → LLM 建议文件夹 → 自动移动文件。最多 20 个文件/次。需调用 LLM。",
              llm: true,
            },
            {
              task: "classify_full", cmd: "nexus_maintain_classify", args: { fullScan: true },
              label: "全量归类",
              desc: "遍历整个 wiki 目录树，对所有文件进行 LLM 分类并归入对应文件夹。深度扫描。需调用 LLM。",
              llm: true,
            },
          ]}
          {...maintProps}
        />

        <MaintGroup
          layer="文档实体提取"
          color="#3a6a3a"
          items={[
            {
              task: "reindex_force", cmd: "nexus_reindex_force",
              label: "强制重新提取",
              desc: "清除提取缓存，对所有 wiki 文档用当前 LLM 配置重新提取实体和关系。不清除旧数据，仅在旧实体上追加/更新。需调用 LLM，文档越多耗时越长。",
              llm: true,
            },
          ]}
          {...maintProps}
        />

        <MaintGroup
          layer="D · 图谱结构分析"
          color="#2B5A2B"
          items={[
            {
              task: "pagerank", cmd: "nexus_run_pagerank",
              label: "PageRank",
              desc: "d=0.85 迭代替换计算每个实体的重要性得分，分别计算文档级和实体级。纯算法，不消耗 token。",
            },
            {
              task: "community", cmd: "nexus_run_community",
              label: "社区检测",
              desc: "Louvain 贪婪算法检测知识图谱中的社区结构，结果用于图谱社区折叠视图。纯算法，不消耗 token。",
            },
          ]}
          {...maintProps}
        />

        <MaintGroup
          layer="E · 关系推导与验证"
          color="#3A2A5A"
          items={[
            {
              task: "transitive", cmd: "nexus_run_transitive",
              label: "传递推理",
              desc: "扫描全部 A→B→C 路径（不限关系类型）→ LLM 判断 A→C 是否合理并确定关系类型和置信度 → 创建推断边（inferred=1）。最多处理 100 条候选路径。需调用 LLM。",
              llm: true,
            },
            {
              task: "verify", cmd: "nexus_verify_synthesis",
              label: "验证合成边",
              desc: "对 inferred=1 的推断边分批提交 LLM 验证其合理性，确认保留/拒绝删除，保证图谱准确度。需调用 LLM。",
              llm: true,
            },
            {
              task: "synthesis", cmd: "nexus_run_synthesis",
              label: "知识合成",
              desc: "基于共享邻居和共现关系推断语义关联，生成新的 inferred=1 关系边。纯规则引擎，不消耗 token。",
            },
          ]}
          {...maintProps}
        />

        <MaintGroup
          layer="F · 冲突与矛盾检测"
          color="#5A3A2A"
          items={[
            {
              task: "conflicts", cmd: "nexus_scan_conflicts",
              label: "扫描冲突",
              desc: "检查预定义互斥对→SQL自连接→BFS检测有向环→LLM审核语义矛盾。阶段1+2纯规则，阶段3需调用LLM。",
              llm: true,
            },
          ]}
          {...maintProps}
        />
      </div>

        {taskResult && (
          <div style={{
            marginTop: 10,
            padding: 10,
            borderRadius: 6,
            fontSize: 12,
            background: taskResult.error ? "rgba(250,81,81,0.1)" : "rgba(7,193,96,0.1)",
            border: `1px solid ${taskResult.error ? "rgba(250,81,81,0.25)" : "rgba(7,193,96,0.25)"}`,
          }}>
            {taskResult.error ? (
              <span style={{ color: "#fa5151" }}>错误: {taskResult.error}</span>
            ) : (
              <div>
                {taskResult.summary && (
                  <div style={{ color: "#07c160", fontWeight: 500, marginBottom: 4 }}>{taskResult.summary}</div>
                )}
                {taskResult.status && !taskResult.summary && (
                  <div style={{ color: "#07c160", fontWeight: 500, marginBottom: 4 }}>状态: {taskResult.status}</div>
                )}
                <CompactReport result={taskResult} />
                {taskResult.details && taskResult.details.length > 0 && (
                  <div style={{ maxHeight: 120, overflow: "auto" }}>
                    {taskResult.details.slice(0, 10).map((d: any, i: number) => (
                      <div key={i} style={{ color: "#b0b0b0", fontSize: 11, marginBottom: 2 }}>
                        {d.action}: {d.entity_name || d.name} — {d.reason || d.message || d.score}
                      </div>
                    ))}
                    {taskResult.details.length > 10 && (
                      <div style={{ color: "#808080", fontSize: 11 }}>...还有 {taskResult.details.length - 10} 条</div>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        )}

      {/* ── Recent Tasks ── */}
      {status && status.recent_tasks && status.recent_tasks.length > 0 && (
        <div className={styles.dividerSm}>
          <div className={styles.label} style={{ marginBottom: 10 }}>最近任务</div>
          {status.recent_tasks.map((t: any, i: number) => (
            <div key={i} style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              padding: "6px 0",
              borderBottom: "1px solid #2C2C2C",
              fontSize: 12,
            }}>
              <span style={{ color: "#e6e6e6" }}>
                <TaskLabel task={t.task} /> {t.summary.slice(0, 40)}
              </span>
              <span style={{ color: t.status === "completed" ? "#07c160" : "#808080", fontSize: 11 }}>
                {t.completed_at ? t.completed_at.slice(0, 10) : t.status}
              </span>
            </div>
          ))}
        </div>
      )}

      {/* ── LLM Config ── */}
      <div className={styles.dividerSm}>
        <div style={{ marginBottom: 12 }}>
          <div style={{ display: "flex", gap: 8, marginBottom: 6 }}>
            <label className={styles.radioRow} onClick={() => handleModeChange("follow_agent")} style={{ display: "inline-flex", marginRight: 16 }}>
              <input type="radio" name="nexus_mode" checked={llmMode === "follow_agent"} readOnly />
              跟随Agent <span className={styles.radioHint}>使用聊天模型</span>
            </label>
            <label className={styles.radioRow} onClick={() => handleModeChange("custom")} style={{ display: "inline-flex" }}>
              <input type="radio" name="nexus_mode" checked={llmMode === "custom"} readOnly />
              独立配置 <span className={styles.radioHint}>单独设置API Key</span>
            </label>
          </div>
          {llmMode === "follow_agent" && (
            <div className={styles.desc} style={{ color: "#07c160" }}>Nexus 将使用 Agent 当前配置的大模型进行知识提取</div>
          )}
        </div>
        {llmMode === "custom" && (<>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 12 }}>
          <div className={styles.label}>独立模型配置</div>
          <button type="button" className={styles.btnPrimary} style={{ padding: "4px 12px", fontSize: 12 }}
            onClick={async () => {
              try {
                const config: any = await invoke("copy_agent_config_for_nexus");
                const updated = {
                  ...nexusConfig,
                  llm_mode: config.llm_mode || "custom",
                  llm_provider: config.llm_provider,
                  llm_model: config.llm_model,
                  llm_api_key: config.llm_api_key,
                  llm_base_url: config.llm_base_url,
                };
                setNexusConfig(updated);
                await invoke("save_nexus_config", { config: updated });
              } catch {}
            }}>
            复制Agent配置
          </button>
        </div>
        <div className={styles.desc} style={{ marginBottom: 10 }}>
          默认独立配置。点击"复制Agent配置"将聊天大模型的密钥一键填入下方。
        </div>
        <NexusProviderRow
          nexusConfig={nexusConfig}
          onFieldChange={handleFieldChange}
          onVerify={(ok, msg) => setTestResult({ ok, model: nexusConfig.llm_model || "?", latency_ms: 0, message: msg })}
        />
        </>)}
      </div>

    </div>
  );
}

function StatusItem({ label, value, warn }: { label: string; value: any; warn?: boolean }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12 }}>
      <span style={{ color: "#808080" }}>{label}</span>
      <span style={{ color: warn ? "#fa9d3b" : "#e6e6e6", fontWeight: warn ? 500 : 400 }}>
        {typeof value === "number" ? value : String(value)}
      </span>
    </div>
  );
}

function TaskLabel({ task }: { task: string }) {
  const labels: Record<string, string> = {
    quality: "质量评分",
    cleanup: "清理",
    dedup: "去重",
    fix_migrated: "迁移修复",
    classify: "文档归类(增量)",
    classify_full: "文档归类(全量)",
    pagerank: "PageRank",
    community: "社区检测",
    causal: "因果链",
    transitive: "传递推理",
    conflicts: "冲突扫描",
    evolution: "演化分析",
    verify: "边验证",
  };
  return <span style={{ color: "#808080" }}>[{labels[task] || task}]</span>;
}

interface MaintItem {
  task: string;
  cmd: string;
  label: string;
  desc: string;
  args?: Record<string, unknown>;
  llm?: boolean;
}

function MaintGroup({
  layer,
  color,
  items,
  runningTask,
  runMaintenance,
}: {
  layer: string;
  color: string;
  items: MaintItem[];
  runningTask: string | null;
  runMaintenance: (task: string, cmd: string, args?: Record<string, unknown>) => void;
}) {
  return (
    <div style={{ marginBottom: 16 }}>
      <div style={{
        fontSize: 11,
        color: "#808080",
        marginBottom: 8,
        paddingLeft: 2,
      }}>
        {layer}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {items.map((item) => (
          <div
            key={item.task}
            style={{
              display: "flex",
              alignItems: "flex-start",
              justifyContent: "space-between",
              gap: 12,
              padding: "10px 12px",
              background: "#2C2C2C",
              border: "1px solid #333333",
              borderRadius: 6,
            }}
          >
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
                <span style={{ fontSize: 12, color: "#e6e6e6", fontWeight: 500 }}>
                  {item.label}
                </span>
                {item.llm && (
                  <span style={{
                    fontSize: 9,
                    padding: "1px 5px",
                    borderRadius: 3,
                    background: "rgba(240, 192, 64, 0.12)",
                    color: "#f0c040",
                  }}>
                    LLM
                  </span>
                )}
              </div>
              <div style={{ fontSize: 11, color: "#808080", lineHeight: 1.5 }}>
                {item.desc}
              </div>
            </div>
            <button
              className={styles.btnSmPrimary}
              style={{
                fontSize: 11,
                flexShrink: 0,
                background: runningTask === item.task ? "#555" : color,
                color: "#fff",
                border: runningTask === item.task ? "1px solid #555" : "1px solid transparent",
                opacity: runningTask !== null && runningTask !== item.task ? 0.4 : 1,
              }}
              onClick={() => runMaintenance(item.task, item.cmd, item.args)}
              disabled={runningTask !== null}
            >
              {runningTask === item.task
                ? "执行中..."
                : item.label}
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

function CompactReport({ result }: { result: any }) {
  const kv: [string, any][] = [];
  if (result.total_entities !== undefined) kv.push(["实体数", result.total_entities]);
  if (result.iterations !== undefined) kv.push(["迭代", result.iterations]);
  if (result.converged !== undefined) kv.push(["收敛", result.converged ? "是" : "否"]);
  if (result.core_count !== undefined) kv.push(["核心数", result.core_count]);
  if (result.communities !== undefined) kv.push(["社区数", result.communities]);
  if (result.modularity !== undefined) kv.push(["模块度", result.modularity?.toFixed(4)]);
  if (result.scanned !== undefined && result.inferred !== undefined) kv.push(["扫描/推断", `${result.scanned}/${result.inferred}`]);
  if (result.skipped_existing !== undefined) kv.push(["跳过已有", result.skipped_existing]);
  if (result.scanned_pairs !== undefined) kv.push(["扫描对", result.scanned_pairs]);
  if (result.conflicts_found !== undefined) kv.push(["冲突数", result.conflicts_found]);
  if (result.total_edges !== undefined) kv.push(["总边数", result.total_edges]);
  if (result.verified !== undefined) kv.push(["验证通过", result.verified]);
  if (result.rejected !== undefined) kv.push(["已拒绝", result.rejected]);
  if (result.entities_scanned !== undefined && !kv.length) kv.push(["扫描实体", result.entities_scanned]);
  if (result.entities_fixed !== undefined && !kv.length) kv.push(["修复数", result.entities_fixed]);

  if (kv.length === 0) return null;
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: "4px 12px", marginBottom: 4 }}>
      {kv.map(([k, v]) => (
        <span key={k} style={{ color: "#b0b0b0", fontSize: 11 }}>{k}: <span style={{ color: "#e6e6e6" }}>{String(v)}</span></span>
      ))}
      {result.top_entities && result.top_entities.length > 0 && (
        <div style={{ width: "100%", marginTop: 4, maxHeight: 100, overflow: "auto" }}>
          <div style={{ color: "#808080", fontSize: 10, marginBottom: 2 }}>Top 实体:</div>
          {result.top_entities.slice(0, 5).map((e: any, i: number) => (
            <div key={i} style={{ color: "#b0b0b0", fontSize: 10, marginBottom: 1 }}>
              {i + 1}. {e.name} ({(e.score || 0).toFixed(6)})
            </div>
          ))}
        </div>
      )}
      {result.conflicts && result.conflicts.length > 0 && (
        <div style={{ width: "100%", marginTop: 4, maxHeight: 100, overflow: "auto" }}>
          <div style={{ color: "#fa9d3b", fontSize: 10, marginBottom: 2 }}>冲突列表:</div>
          {result.conflicts.slice(0, 5).map((c: any, i: number) => (
            <div key={i} style={{ color: "#b0b0b0", fontSize: 10, marginBottom: 1 }}>
              {c.entity_a} [{c.relation_a}] vs [{c.relation_b}] → {c.target}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function TasksSection() {
  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [loading, setLoading] = useState(true);
  const [actionMsg, setActionMsg] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [showOutput, setShowOutput] = useState<string | null>(null);
  const [outputLines, setOutputLines] = useState<string[]>([]);

  const load = useCallback(async () => {
    try { setJobs(await listCronJobs()); } catch { /* */ }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  const doAction = async (fn: () => Promise<unknown>, msg: string) => {
    try { await fn(); setActionMsg(msg); load(); }
    catch (e: any) { setActionMsg(`错误: ${typeof e === "string" ? e : e?.message}`); }
    setTimeout(() => setActionMsg(null), 3000);
  };

  const handleShowOutput = async (jobId: string) => {
    if (showOutput === jobId) { setShowOutput(null); return; }
    try {
      const lines = await getCronOutput(jobId);
      setOutputLines(lines);
      setShowOutput(jobId);
    } catch { setOutputLines(["读取日志失败"]); setShowOutput(jobId); }
  };

  if (loading) return <div className={styles.section}><div className={styles.desc}>加载中...</div></div>;

  return (
    <div className={styles.section}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
        <h2 className={styles.sectionTitle}>定时任务</h2>
        <button className={styles.btnSmPrimary} onClick={() => setShowAdd(true)}>+ 新建任务</button>
      </div>
      <div className={styles.desc} style={{ marginBottom: 16 }}>
        Agent 网关内置的 cron 调度引擎每 60 秒检查一次，自动执行到期的 LLM 任务。
      </div>

      {actionMsg && (
        <div style={{ padding: "6px 12px", marginBottom: 10, borderRadius: 6, fontSize: 12,
          background: actionMsg.startsWith("错误") ? "rgba(250,81,81,0.1)" : "rgba(7,193,96,0.1)",
          border: `1px solid ${actionMsg.startsWith("错误") ? "rgba(250,81,81,0.25)" : "rgba(7,193,96,0.25)"}`,
          color: actionMsg.startsWith("错误") ? "#fa5151" : "#07c160", }}>
          {actionMsg}
        </div>
      )}

      {/* ── System tasks ── */}
      <div className={styles.label} style={{ marginBottom: 8 }}>系统后台任务</div>
      <div className={styles.card} style={{ marginBottom: 10, padding: "10px 14px", display: "flex", alignItems: "center", justifyContent: "space-between" }}>
        <div>
          <span style={{ fontSize: 13, color: "#e6e6e6", fontWeight: 500 }}>Agent 健康监控</span>
          <div className={styles.desc}>每 30 秒检测网关 /health，异常时自动重启（最多 5 次）</div>
        </div>
        <span className={styles.badgeGreen}>自动运行</span>
      </div>
      <div className={styles.card} style={{ marginBottom: 10, padding: "10px 14px", display: "flex", alignItems: "center", justifyContent: "space-between" }}>
        <div>
          <span style={{ fontSize: 13, color: "#e6e6e6", fontWeight: 500 }}>Wiki 文件监听</span>
          <div className={styles.desc}>监听文档目录变化，自动提取实体 + 清理失效引用</div>
        </div>
        <span className={styles.badgeGreen}>自动运行</span>
      </div>

      {/* ── Cron jobs ── */}
      <div className={styles.label} style={{ marginBottom: 8, marginTop: 18 }}>
        LLM 定时任务 ({jobs.length})
      </div>
      {jobs.length === 0 ? (
        <div className={styles.desc} style={{ marginBottom: 12 }}>暂无任务。点击"新建任务"创建一个基于 LLM 的定时任务。</div>
      ) : (
        jobs.map((job) => (
          <div key={job.id} className={styles.card} style={{
            marginBottom: 8, padding: "10px 14px",
            display: "flex", alignItems: "flex-start", justifyContent: "space-between",
            opacity: job.enabled ? 1 : 0.5,
          }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 2 }}>
                <span style={{ fontSize: 13, color: job.enabled ? "#e6e6e6" : "#808080", fontWeight: 500 }}>
                  {job.name || "未命名"}
                </span>
                <span className={job.enabled ? styles.badgeGreen : styles.badgeGray}>
                  {job.enabled ? "运行中" : "已暂停"}
                </span>
              </div>
              <div className={styles.desc}>
                频率: {job.schedule_display || "—"}
                {job.deliver && job.deliver !== "local" ? ` | 投递: ${job.deliver}` : ""}
                {(job.skills && job.skills.length > 0) ? ` | Skills: ${job.skills.join(", ")}` : ""}
              </div>
              <div className={styles.desc}>
                上次: {job.last_run_at ? `${job.last_run_at.slice(0, 16)} (${job.last_status || "?"})` : "从未"}
                {job.last_error ? ` — ${job.last_error.slice(0, 80)}` : ""}
                {job.next_run_at ? ` | 下次: ${job.next_run_at.slice(0, 16)}` : ""}
              </div>
              {/* Output panel */}
              {showOutput === job.id && (
                <div style={{ marginTop: 8, padding: "8px 10px", background: "#1A1A1A", borderRadius: 4, fontSize: 11, color: "#b3b3b3", maxHeight: 160, overflow: "auto" }}>
                  {outputLines.length === 0 ? <span>无输出记录</span> : outputLines.map((l, i) => <pre key={i} style={{ margin: 0, whiteSpace: "pre-wrap", fontFamily: "monospace" }}>{l}</pre>)}
                </div>
              )}
            </div>
            <div style={{ display: "flex", gap: 4, alignItems: "center", flexShrink: 0, marginLeft: 8 }}>
              <button className={styles.btnSmMuted} style={{ fontSize: 11 }} onClick={() => handleShowOutput(job.id)}>
                {showOutput === job.id ? "收起" : "日志"}
              </button>
              <button className={styles.btnSmPrimary} style={{ fontSize: 11 }}
                onClick={() => doAction(() => triggerCronJob(job.id), `已触发「${job.name}」`)}>
                触发
              </button>
              <button className={job.enabled ? styles.btnSmYellow : styles.btnSmPrimary} style={{ fontSize: 11 }}
                onClick={() => doAction(() => toggleCronJob(job.id, !job.enabled), `已${job.enabled ? "暂停" : "恢复"}「${job.name}」`)}>
                {job.enabled ? "暂停" : "恢复"}
              </button>
              <button className={styles.btnSmDanger} style={{ fontSize: 11 }}
                onClick={() => { if (confirm(`删除任务「${job.name}」？`)) doAction(() => deleteCronJob(job.id), `已删除「${job.name}」`); }}>
                删除
              </button>
            </div>
          </div>
        ))
      )}

      {/* Add job dialog */}
      {showAdd && <AddCronDialog onClose={() => setShowAdd(false)} onAdded={() => { setShowAdd(false); load(); }} />}
    </div>
  );
}

function AddCronDialog({ onClose, onAdded }: { onClose: () => void; onAdded: () => void }) {
  const [name, setName] = useState("");
  const [schedule, setSchedule] = useState("0 9 * * *");
  const [prompt, setPrompt] = useState("");
  const [deliver, setDeliver] = useState("local");
  const [skills, setSkills] = useState("");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState("");

  const handleAdd = async () => {
    if (!name.trim()) { setErr("请输入任务名称"); return; }
    if (!schedule.trim()) { setErr("请输入调度表达式"); return; }
    setSaving(true); setErr("");
    try {
      await addCronJob({
        name: name.trim(),
        schedule: schedule.trim(),
        prompt: prompt.trim() || null,
        deliver: deliver || "local",
        skills: skills ? skills.split(",").map((s) => s.trim()).filter(Boolean) : [],
      });
      onAdded();
    } catch (e: any) {
      setErr(typeof e === "string" ? e : e?.message || "创建失败");
    } finally { setSaving(false); }
  };

  const presets = [
    { label: "每天 09:00", expr: "0 9 * * *" },
    { label: "每天 18:00", expr: "0 18 * * *" },
    { label: "每小时", expr: "0 * * * *" },
    { label: "每 30 分钟", expr: "*/30 * * * *" },
    { label: "每周一 08:00", expr: "0 8 * * 1" },
  ];

  return (
    <div style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.6)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000 }}
      onClick={onClose}>
      <div style={{ background: "#1e1e1e", border: "1px solid #444", borderRadius: 12, padding: 24, width: 480, maxHeight: "90vh", overflow: "auto" }}
        onClick={(e) => e.stopPropagation()}>
        <h3 style={{ margin: "0 0 16px 0", fontSize: 16 }}>新建 LLM 定时任务</h3>

        <div className={styles.fieldGroup}>
          <label className={styles.label}>任务名称</label>
          <input className={styles.textInput} value={name} onChange={(e) => setName(e.target.value)} placeholder="如：每日早报摘要" />
        </div>

        <div className={styles.fieldGroup}>
          <label className={styles.label}>调度表达式</label>
          <input className={styles.textInput} value={schedule} onChange={(e) => setSchedule(e.target.value)} placeholder="0 9 * * *" />
          <div style={{ display: "flex", gap: 4, marginTop: 4, flexWrap: "wrap" }}>
            {presets.map((p) => (
              <button key={p.expr} type="button" onClick={() => setSchedule(p.expr)}
                style={{ fontSize: 10, padding: "2px 6px", borderRadius: 4, border: "1px solid #444", background: schedule === p.expr ? "#3b5fd9" : "transparent", color: schedule === p.expr ? "#fff" : "#888", cursor: "pointer" }}>
                {p.label}
              </button>
            ))}
          </div>
        </div>

        <div className={styles.fieldGroup}>
          <label className={styles.label}>LLM 提示词</label>
          <textarea className={styles.textInput} value={prompt} onChange={(e) => setPrompt(e.target.value)}
            placeholder="输入给 Agent 的指令，如：检查知识库质量并生成报告"
            rows={1}
            style={{
              resize: "none", overflowY: "auto", minHeight: 36,
              maxHeight: 110, lineHeight: 1.5,
            }}
            onInput={(e) => {
              const el = e.currentTarget;
              el.style.height = "auto";
              el.style.height = Math.min(el.scrollHeight, 110) + "px";
            }} />
        </div>

        <div className={styles.fieldGroup}>
          <label className={styles.label}>投递目标（可选）</label>
          <select className={styles.textInput} value={deliver} onChange={(e) => setDeliver(e.target.value)}>
            <option value="local">本地（仅保存日志）</option>
            <option value="weixin">微信</option>
            <option value="wecom">企业微信</option>
            <option value="feishu">飞书</option>
            <option value="dingtalk">钉钉</option>
            <option value="telegram">Telegram</option>
            <option value="discord">Discord</option>
            <option value="slack">Slack</option>
            <option value="whatsapp">WhatsApp</option>
          </select>
        </div>

        <div className={styles.fieldGroup}>
          <label className={styles.label}>Skills（逗号分隔，可选）</label>
          <input className={styles.textInput} value={skills} onChange={(e) => setSkills(e.target.value)}
            placeholder="如：web-search, memory, file" />
        </div>

        {err && <div style={{ color: "#fa5151", fontSize: 12, marginTop: 8 }}>{err}</div>}

        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end", marginTop: 16 }}>
          <button className={styles.btnSmMuted} onClick={onClose}>取消</button>
          <button className={styles.btnSmPrimary} onClick={handleAdd} disabled={saving}>
            {saving ? "创建中..." : "创建任务"}
          </button>
        </div>
      </div>
    </div>
  );
}

function SectionContent({ section }: { section: SettingsSection }) {
  switch (section) {
    case "account": return <AccountSection />;
    case "gateway": return <GatewaySection />;
    case "theme": return <ThemeSection />;
    case "language": return <LanguageSection />;
    case "voice": return <VoiceSection />;
    case "update": return <UpdateSection />;
    case "migration": return <MigrationSection />;
    case "tasks": return <TasksSection />;
    case "nexus": return <NexusSection />;
    case "agents": return <AgentSettings />;
  }
}

export default function SettingsPage() {
  const [activeSection, setActiveSection] = useState<SettingsSection>("account");

  return (
    <div className={styles.page}>
      <aside className={styles.sidebar}>
        <div className={styles.sidebarTitle}>设置</div>
        {navItems.map((item) => (
          <button
            key={item.id}
            className={`${styles.navItem} ${activeSection === item.id ? styles.navActive : ""}`}
            onClick={() => setActiveSection(item.id)}
          >
            {item.label}
          </button>
        ))}
      </aside>
      <main className={styles.content}>
        <SectionContent section={activeSection} />
      </main>
    </div>
  );
}

// ── Nexus Provider Row (knowledge engine custom LLM) ──
const NEXUS_PROVIDERS: { id: string; label: string; url: string; model: string }[] = [
  { id: "deepseek", label: "DeepSeek", url: "https://api.deepseek.com", model: "deepseek-v4-pro" },
  { id: "aigocode", label: "AIGoCode", url: "https://api.aigocode.com", model: "gpt-5.4" },
  { id: "openai", label: "OpenAI", url: "https://api.openai.com", model: "gpt-4o" },
  { id: "anthropic", label: "Anthropic", url: "https://api.anthropic.com", model: "claude-sonnet-4-6" },
  { id: "google", label: "Google AI", url: "https://generativelanguage.googleapis.com", model: "gemini-2.5-flash" },
  { id: "xai", label: "xAI", url: "https://api.x.ai", model: "grok-3" },
  { id: "groq", label: "Groq", url: "https://api.groq.com/openai", model: "llama-4-maverick" },
  { id: "openrouter", label: "OpenRouter", url: "https://openrouter.ai/api", model: "openai/gpt-4o" },
  { id: "ollama", label: "Ollama (本地)", url: "http://localhost:11434/v1", model: "" },
  { id: "lmstudio", label: "LM Studio (本地)", url: "http://localhost:1234/v1", model: "" },
];

function NexusProviderRow({ nexusConfig, onFieldChange, onVerify }: {
  nexusConfig: any;
  onFieldChange: (key: string, value: string) => void;
  onVerify: (ok: boolean, msg: string) => void;
}) {
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<{ok: boolean; msg: string} | null>(null);
  const sel = NEXUS_PROVIDERS.find(p => p.url === (nexusConfig.llm_base_url || ""));
  const provider = sel?.id || nexusConfig.llm_provider || "";

  const handleVerify = async () => {
    console.log("[nexus-verify] nexusConfig:", nexusConfig);
    const url = (nexusConfig?.llm_base_url || "").trim();
    const key = (nexusConfig?.llm_api_key || "").trim();
    const model = (nexusConfig?.llm_model || "").trim();
    console.log("[nexus-verify] url:", url, "key:", key ? "***" : "(empty)", "model:", model);
    if (!url) { setResult({ok: false, msg: "请先选择提供商"}); return; }
    if (!model) { setResult({ok: false, msg: "请填写模型名称"}); return; }
    // Local providers don't need API key
    const isLocal = url.includes("localhost") || url.includes("127.0.0.1");
    if (!key && !isLocal) { setResult({ok: false, msg: "请填写API Key"}); return; }
    setChecking(true); setResult({ok: false, msg: "正在验证..."});
    try {
      const msg = await invoke<string>("verify_api_key", { baseUrl: url, apiKey: key, model });
      setResult({ok: true, msg: `验证成功: ${msg}`});
    } catch (e: any) {
      const err = typeof e === "string" ? e : (e?.message || e?.toString?.() || "验证失败");
      console.error("[nexus-verify] error:", e);
      setResult({ok: false, msg: `验证失败: ${err}`});
    }
    setChecking(false);
  };

  return (
    <div style={{ marginBottom: 10 }}>
      <div className={styles.fieldGroup}>
        <label className={styles.label}>提供商</label>
        <select className={styles.textInput}
          value={nexusConfig.llm_base_url || ""}
          onChange={(e) => {
            const p = NEXUS_PROVIDERS.find(x => x.url === e.target.value);
            onFieldChange("llm_base_url", e.target.value || "");
            if (p) {
              onFieldChange("llm_provider", p.id);
              onFieldChange("llm_model", p.model);
            }
          }}>
          <option value="">自定义</option>
          {NEXUS_PROVIDERS.map(p => (
            <option key={p.id} value={p.url}>{p.label} — {p.url}</option>
          ))}
        </select>
      </div>
      <div className={styles.fieldGroup}>
        <label className={styles.label}>API Key</label>
        <PasswordInput
          className={styles.textInput}
          value={nexusConfig.llm_api_key || ""}
          onChange={(v) => { onFieldChange("llm_api_key", v); }}
          placeholder="sk-..."
        />
      </div>
      <div className={styles.fieldGroup}>
        <label className={styles.label}>Model</label>
        <input className={styles.textInput}
          value={nexusConfig.llm_model || ""}
          onChange={(e) => { onFieldChange("llm_model", e.target.value); }}
          placeholder="deepseek-v4-pro" />
      </div>
      <div style={{ display: "flex", gap: 8, marginTop: 8, alignItems: "center" }}>
        <button type="button" className={styles.btnPrimary} onClick={handleVerify} disabled={checking}
          style={{ padding: "4px 12px", fontSize: 12 }}>
          {checking ? "验证中..." : "验证连接"}
        </button>
        <button type="button" className={styles.btnPrimary} onClick={async () => {
          const cleared = { llm_mode: nexusConfig?.llm_mode || "custom", llm_base_url: "", llm_api_key: "", llm_model: "", llm_provider: "" };
          try {
            await invoke("save_nexus_config", { config: cleared });
            onFieldChange("llm_base_url", "");
            onFieldChange("llm_api_key", "");
            onFieldChange("llm_model", "");
            onFieldChange("llm_provider", "");
            setResult({ok: true, msg: "已重置，可配置新模型"});
          } catch(e: any) { setResult({ok: false, msg: String(e)}); }
        }} style={{ padding: "4px 12px", fontSize: 12, background: "#555" }}>
          重置
        </button>
      </div>
      {result && (
        <div style={{ marginTop: 6, fontSize: 12, color: result.ok ? "#07c160" : "#f87171" }}>{result.msg}</div>
      )}
    </div>
  );
}
