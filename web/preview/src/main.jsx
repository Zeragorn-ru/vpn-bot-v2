import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

const navItems = [
  ["overview", "Overview", "01"],
  ["customers", "Customers", "02"],
  ["subscriptions", "Subscriptions", "03"],
  ["settings", "Runtime settings", "04"],
];
const format = (bytes) => `${(bytes / 1e9).toFixed(1)} GB`;

function Brand() { return <div className="brand"><span className="brand-mark">V</span><span>VECTOR</span></div>; }
function RouteTabs({ active }) { return <nav className="route-tabs" aria-label="Preview pages"><a className={active === "mini" ? "active" : ""} href="/mini">Mini App</a><a className={active === "admin" ? "active" : ""} href="/admin">Admin</a><a className={active === "sub" ? "active" : ""} href="/sub/demo">Aggregator</a></nav>; }
function Badge({ children, tone = "green" }) { return <span className={`badge ${tone}`}>{children}</span>; }
function Progress({ value }) { return <div className="progress"><span style={{ width: `${value}%` }} /></div>; }
function Button({ children, secondary = false, onClick }) { return <button onClick={onClick} className={secondary ? "button secondary" : "button"}>{children}</button>; }
function Toast({ message, onClose }) { return message ? <div className="toast"><span className="toast-pulse" />{message}<button onClick={onClose}>×</button></div> : null; }

function MiniApp() {
  const [copied, setCopied] = useState(false);
  const [sheet, setSheet] = useState(null);
  const link = "https://test.zeragorn.xyz/sub/demo";
  const copyLink = async () => { try { await navigator.clipboard.writeText(link); } catch {} setCopied(true); setTimeout(() => setCopied(false), 1600); };
  return <div className="app-frame mini-page">
    <header className="topbar"><Brand /><RouteTabs active="mini" /><button className="avatar" aria-label="Open profile">Z</button></header>
    <main className="mini-content">
      <section className="eyebrow-row"><span className="eyebrow">PERSONAL ROUTE / 01</span><Badge>online</Badge></section>
      <h1>Private internet,<br /><em>without the noise.</em></h1>
      <p className="lede">A calm, fast connection for the places you go every day.</p>
      <section className="hero-card">
        <div className="route-map" aria-hidden="true"><span className="map-line line-a" /><span className="map-line line-b" /><i className="map-node node-a" /><i className="map-node node-b" /><i className="map-node node-c" /><span className="map-label label-a">AMS / 01</span><span className="map-label label-b">YOU</span></div>
        <div className="hero-card-head"><span>YOUR ACCESS</span><Badge>active</Badge></div>
        <strong className="hero-title">VECTOR / ONE</strong>
        <div className="hero-meta"><div><small>VALID UNTIL</small><b>24 AUG 2026</b></div><div><small>LOCATION</small><b>NL · AMSTERDAM</b></div></div>
        <div className="usage"><div><small>TRAFFIC USED</small><b>35.0 <i>/ 100 GB</i></b></div><span>35%</span></div><Progress value={35} />
        <Button onClick={() => setSheet("connect")}>Open subscription <span>↗</span></Button>
      </section>
      <section className="section-heading"><span>QUICK ACTIONS</span><a href="/sub/demo">View all ↗</a></section>
      <div className="action-grid"><button className="action-card" onClick={() => setSheet("connect")}><span className="action-icon lime">↗</span><b>Connect a device</b><small>Copy your private link</small></button><button className="action-card" onClick={() => setSheet("plans")}><span className="action-icon violet">＋</span><b>Extend access</b><small>Plans from ₽299</small></button></div>
      <section className="section-heading"><span>YOUR LINK</span><span className="muted">ACCESS TOKEN</span></section>
      <div className="copy-card"><code>{link}</code><button onClick={copyLink}>{copied ? "Copied ✓" : "Copy"}</button></div>
    </main>
    <footer className="mobile-nav"><a className="selected" href="/mini"><span>⌂</span>Home</a><button onClick={() => setSheet("plans")}><span>◈</span>Plans</button><button onClick={() => setSheet("support")}><span>?</span>Support</button></footer>
    {sheet && <div className="sheet-backdrop" onClick={() => setSheet(null)}><section className="bottom-sheet" onClick={(event) => event.stopPropagation()}><button className="sheet-close" onClick={() => setSheet(null)}>×</button>{sheet === "connect" && <><span className="eyebrow">CONNECT A DEVICE</span><h2>One link.<br /><em>Every client.</em></h2><p>Copy the private subscription URL and paste it into Clash, sing-box, Xray or your preferred client.</p><div className="sheet-link"><code>{link}</code><button onClick={copyLink}>{copied ? "Copied" : "Copy"}</button></div><Button onClick={() => window.location.href = "/sub/demo"}>Open aggregator ↗</Button></>}{sheet === "plans" && <><span className="eyebrow">EXTEND ACCESS</span><h2>Stay on<br /><em>your route.</em></h2><div className="plan-options"><button><b>1 month</b><span>₽299</span><small>100 GB · instant</small></button><button className="plan-featured"><b>3 months</b><span>₽799</span><small>300 GB · save 11%</small></button></div></>}{sheet === "support" && <><span className="eyebrow">SUPPORT</span><h2>We are<br /><em>on route.</em></h2><p>Send us a message any time. A human will answer within a few minutes.</p><Button onClick={() => { window.location.href = "mailto:support@test.zeragorn.xyz"; }}>Contact support ↗</Button></>}</section></div>}
  </div>;
}

function Admin() {
  const [selected, setSelected] = useState("overview");
  const [toast, setToast] = useState("");
  const selectedLabel = navItems.find(([id]) => id === selected)?.[1] || "Overview";
  const choose = (id) => { setSelected(id); setToast(`${navItems.find(([key]) => key === id)?.[1]} view selected`); setTimeout(() => setToast(""), 1800); };
  return <div className="admin-page">
    <aside className="sidebar"><Brand /><div className="workspace"><span className="workspace-dot" />VECTOR / LAB <span>⌄</span></div><div className="side-nav">{navItems.map(([id, label, no]) => <button key={id} className={selected === id ? "selected" : ""} onClick={() => choose(id)}><span>{no}</span>{label}</button>)}</div><div className="side-bottom"><div className="system-state"><i />All systems nominal</div><div className="user-row"><span className="avatar">Z</span><div><b>zeragorn</b><small>Owner</small></div><span>•••</span></div></div></aside>
    <main className="admin-main"><Toast message={toast} onClose={() => setToast("")} /><header className="admin-header"><div><span className="eyebrow">TUESDAY, 08 SEPTEMBER 2026 · {selectedLabel.toUpperCase()}</span><h1>Good evening, <em>Zeragorn.</em></h1></div><div className="header-actions"><button className="icon-button" onClick={() => setToast("Search is ready for live data")}>⌕</button><button className="icon-button" onClick={() => setToast("No new alerts")}>◌</button><button className="button compact" onClick={() => setToast("Action menu opened")}>+ New action</button></div></header>
      <div className="admin-grid"><section className="stat-panel wide"><div className="panel-label">RECURRING REVENUE <span>↗ 12.4%</span></div><strong>₽ 284,600</strong><div className="sparkline"><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/><i/></div><div className="chart-axis"><span>01 AUG</span><span>08 SEP</span></div></section><section className="stat-panel"><div className="panel-label">ACTIVE ROUTES</div><strong>1,284</strong><small className="stat-foot"><span className="green-dot" /> +8.6% this month</small></section><section className="stat-panel"><div className="panel-label">PROVISIONING</div><strong>99.97<span className="unit">%</span></strong><small className="stat-foot"><span className="green-dot" /> 34 ms median</small></section></div>
      <div className="content-grid"><section className="panel queue-panel"><div className="panel-title"><div><span className="eyebrow">LIVE OPERATIONS</span><h2>Provisioning queue</h2></div><button className="panel-link" onClick={() => setToast("Queue is up to date")}>View queue ↗</button></div><div className="queue-item"><span className="queue-icon lime">+</span><div><b>New subscription · VECTOR / ONE</b><small>user_2184 · 12 seconds ago</small></div><Badge>completed</Badge></div><div className="queue-item"><span className="queue-icon violet">↻</span><div><b>Route refresh · Amsterdam</b><small>provider/remnawave · 2 minutes ago</small></div><Badge tone="blue">syncing</Badge></div><div className="queue-item"><span className="queue-icon">✓</span><div><b>Link rotated · user_1940</b><small>operator · 14 minutes ago</small></div><Badge tone="gray">logged</Badge></div></section><section className="panel health-panel"><div className="panel-title"><div><span className="eyebrow">SYSTEM HEALTH</span><h2>Everything is clear</h2></div><span className="health-ring">98</span></div><div className="health-row"><span><i className="green-dot" />API</span><b>operational</b><small>24 ms</small></div><div className="health-row"><span><i className="green-dot" />Provider</span><b>operational</b><small>42 ms</small></div><div className="health-row"><span><i className="green-dot" />Last backup</span><b>verified</b><small>2h ago</small></div><div className="backup-note"><span>↗</span><div><b>Next backup in 04:18:22</b><small>S3 · eu-central-1 · encrypted</small></div></div></section></div>
      <section className="panel lower-panel"><div className="panel-title"><div><span className="eyebrow">AGGREGATOR</span><h2>Public experience</h2></div><a className="panel-link" href="/sub/demo">Open preview ↗</a></div><div className="aggregator-preview"><div className="preview-swatch"><span className="brand-mark">V</span></div><div><b>test.zeragorn.xyz</b><small>Browser page + 4 client formats</small></div><Badge>published</Badge><span className="arrow">→</span></div></section>
    </main>
  </div>;
}

function Aggregator() {
  const [formatName, setFormatName] = useState("Clash / Mihomo");
  const [copied, setCopied] = useState(false);
  const formats = [["Clash / Mihomo", "YAML profile", "C", ""], ["sing-box", "JSON profile", "S", "violet-text"], ["Xray", "JSON profile", "X", "blue-text"], ["Generic", "Base64 list", "◉", ""]];
  const copy = async () => { try { await navigator.clipboard.writeText("https://test.zeragorn.xyz/sub/demo"); } catch {} setCopied(true); setTimeout(() => setCopied(false), 1600); };
  return <div className="app-frame aggregator-page"><header className="topbar"><Brand /><RouteTabs active="sub" /><a className="support-link" href="mailto:support@test.zeragorn.xyz">Support ↗</a></header><main className="aggregator-content"><div className="sub-intro"><Badge>subscription active</Badge><span className="eyebrow">VECTOR / PERSONAL ROUTE</span></div><h1>Your private route<br /><em>is ready.</em></h1><p className="lede">One link for every device. Choose a format below or scan the code in your VPN client.</p><section className="sub-card"><div className="sub-card-top"><div><small>PLAN</small><strong>VECTOR / ONE</strong></div><div className="sub-status"><i />Connected</div></div><div className="sub-stats"><div><small>EXPIRES</small><b>24 Aug 2026</b></div><div><small>TRAFFIC</small><b>{format(35e9)} <i>/ {format(100e9)}</i></b></div><div><small>REGION</small><b>NL · AMS</b></div></div><div className="usage"><span>35% used</span><span>65 GB remaining</span></div><Progress value={35} /></section><section className="format-section"><div className="section-heading"><span>CHOOSE YOUR CLIENT</span><span className="muted">{formatName} selected</span></div><div className="format-grid">{formats.map(([name, detail, icon, tone]) => <button key={name} onClick={() => setFormatName(name)} className={`format-card ${formatName === name ? "selected" : ""}`}><span className={`format-logo ${tone}`}>{icon}</span><b>{name}</b><small>{detail}</small><span className="format-arrow">↗</span></button>)}</div></section><section className="link-panel"><div><span className="eyebrow">SUBSCRIPTION URL</span><code>https://test.zeragorn.xyz/sub/demo</code></div><Button onClick={copy}>{copied ? "Copied ✓" : "Copy link"}</Button></section><p className="privacy-note">This page never exposes provider credentials. Your access link is private — do not share it publicly.</p></main></div>;
}

function App() { const path = window.location.pathname; if (path.startsWith("/admin")) return <Admin />; if (path.startsWith("/sub")) return <Aggregator />; return <MiniApp />; }
createRoot(document.getElementById("root")).render(<StrictMode><App /></StrictMode>);
