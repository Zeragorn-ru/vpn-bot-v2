import { useState } from "react";

export default function Login({ setupRequired, error, pending, onSubmit }) {
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [localError, setLocalError] = useState("");

  const submit = (event) => {
    event.preventDefault();
    setLocalError("");
    if (setupRequired && password !== confirm) {
      setLocalError("Пароли не совпадают.");
      return;
    }
    onSubmit({ login, password });
  };

  return (
    <main className="login">
      <section className="login-card">
        <div className="login-brand">
          <b aria-hidden="true" />
          <span>
            VPN <small>CONTROL PLANE</small>
          </span>
        </div>
        <h1>{setupRequired ? "Создание администратора" : "Вход администратора"}</h1>
        <p className="muted-text">
          {setupRequired
            ? "Эта учётная запись получит полный доступ к панели управления."
            : "Используйте логин и пароль администратора."}
        </p>
        <form onSubmit={submit}>
          <label className="field">
            <span>Логин</span>
            <input value={login} autoComplete="username" minLength={3} onChange={(event) => setLogin(event.target.value)} required />
          </label>
          <label className="field">
            <span>Пароль</span>
            <input
              value={password}
              type="password"
              autoComplete={setupRequired ? "new-password" : "current-password"}
              minLength={12}
              onChange={(event) => setPassword(event.target.value)}
              required
            />
          </label>
          {setupRequired && (
            <label className="field">
              <span>Повторите пароль</span>
              <input
                value={confirm}
                type="password"
                autoComplete="new-password"
                minLength={12}
                onChange={(event) => setConfirm(event.target.value)}
                required
              />
            </label>
          )}
          <button type="submit" className="btn primary block" disabled={pending}>
            {pending ? "Проверяем..." : setupRequired ? "Создать администратора" : "Войти"}
          </button>
        </form>
        {(localError || error) && (
          <p className="login-error" role="alert">
            {localError || error}
          </p>
        )}
        <p className="login-foot">Минимальная длина пароля — 12 символов. Действия в панели фиксируются в журнале аудита.</p>
      </section>
    </main>
  );
}
