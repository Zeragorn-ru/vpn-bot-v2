export default function Notice({ message, onClose }) {
  if (!message) return null;
  return (
    <button className="mini-notice" onClick={onClose}>
      {message}
      <b>×</b>
    </button>
  );
}
