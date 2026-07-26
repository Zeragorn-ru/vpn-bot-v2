import { haptic } from "../lib/tg";

export default function ScreenHead({ label, title, onBack }) {
  return (
    <header className="screen-head">
      <button
        onClick={() => {
          haptic("light");
          onBack();
        }}
      >
        ←
      </button>
      <div>
        <p>{label}</p>
        <h1>{title}</h1>
      </div>
    </header>
  );
}
