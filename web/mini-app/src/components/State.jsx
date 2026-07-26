import { haptic } from "../lib/tg";

export default function State({ title, action, actionLabel, error }) {
  return (
    <main className="mini-shell state-screen">
      <span className={error ? "state-mark error" : "state-mark"}>
        {error ? "!" : ""}
      </span>
      <h1>{title}</h1>
      {action && (
        <button
          className="primary"
          onClick={() => {
            haptic("medium");
            action();
          }}
        >
          {actionLabel}
        </button>
      )}
    </main>
  );
}
