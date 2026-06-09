import { Component } from "react";
import AppShell from "./components/layout/AppShell";
import { SplashScreen } from "./components/auth/SplashScreen";
import { LoginForm } from "./components/auth/LoginForm";
import { RegisterForm } from "./components/auth/RegisterForm";
import { ApiSetupWizard } from "./components/auth/ApiSetupWizard";
import { useAuthStore } from "./stores/authStore";
import "./App.css";

class ErrorBoundary extends Component<{ children: React.ReactNode }, { error: Error | null }> {
  constructor(props: any) {
    super(props);
    this.state = { error: null };
  }
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 40, color: "red", fontFamily: "monospace", whiteSpace: "pre-wrap" }}>
          <h1>App Error</h1>
          <pre>{this.state.error.message}</pre>
          <pre>{this.state.error.stack}</pre>
        </div>
      );
    }
    return this.props.children;
  }
}

function App() {
  const stage = useAuthStore((s) => s.stage);
  const setStage = useAuthStore((s) => s.setStage);
  const proceedAfterSplash = useAuthStore((s) => s.proceedAfterSplash);

  if (stage === "splash") {
    return <SplashScreen onComplete={proceedAfterSplash} />;
  }

  if (stage === "register") {
    return <RegisterForm onSwitchToLogin={() => setStage("login")} />;
  }

  if (stage === "login") {
    return <LoginForm onSwitchToRegister={() => setStage("register")} />;
  }

  if (stage === "api_setup") {
    return <ApiSetupWizard />;
  }

  return (
    <ErrorBoundary>
      <AppShell />
    </ErrorBoundary>
  );
}

export default App;
