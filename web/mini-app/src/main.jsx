import React from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

function App() {
  return <main><p className="kicker">VECTOR / PRIVATE ROUTE</p><h1>Internet,<br /><em>without noise.</em></h1><p>Your Mini App will show subscriptions, usage, plans and support after Telegram authentication is configured.</p><button>Open subscription</button></main>;
}
createRoot(document.getElementById("root")).render(<App />);
