import { useEffect, useMemo, useRef, useState } from "react";
import { percent } from "../lib/format.js";
import { Sparkline } from "./charts.jsx";

export function PageHeader({ kicker, title, description, actions }) {
  return (
    <header className="page-head">
      <div>
        {kicker && <p className="eyebrow">{kicker}</p>}
        <h1>{title}</h1>
        {description && <p className="page-desc">{description}</p>}
      </div>
      {actions && <div className="page-actions">{actions}</div>}
    </header>
  );
}

export function Card({ title, description, action, children, className = "", padded = true }) {
  return (
    <section className={`card ${className}`}>
      {(title || action) && (
        <div className="card-head">
          <div>
            {title && <h2>{title}</h2>}
            {description && <p>{description}</p>}
          </div>
          {action}
        </div>
      )}
      <div className={padded ? "card-body" : "card-body flush"}>{children}</div>
    </section>
  );
}

export function Badge({ tone = "muted", children }) {
  return <span className={`badge ${tone}`}>{children}</span>;
}

export function Delta({ value, invert = false }) {
  if (value == null) return <span className="delta neutral">нет базы</span>;
  const positive = invert ? value < 0 : value > 0;
  const tone = value === 0 ? "neutral" : positive ? "up" : "down";
  return (
    <span className={`delta ${tone}`}>
      {value === 0 ? "→" : value > 0 ? "↑" : "↓"} {percent(Math.abs(value))}
    </span>
  );
}

export function MetricCard({ label, value, caption, delta, invertDelta, trend, color, icon }) {
  return (
    <article className="metric">
      <div className="metric-head">
        <span className="eyebrow">{label}</span>
        {icon && <span className="metric-icon">{icon}</span>}
      </div>
      <strong className="metric-value">{value}</strong>
      <div className="metric-foot">
        {delta !== undefined && <Delta value={delta} invert={invertDelta} />}
        {caption && <span className="metric-caption">{caption}</span>}
      </div>
      {trend && <Sparkline values={trend} color={color} />}
    </article>
  );
}

export function Loader({ title = "Загружаем данные", detail }) {
  return (
    <div className="state" role="status">
      <i className="spinner" />
      <h2>{title}</h2>
      {detail && <p>{detail}</p>}
    </div>
  );
}

export function ErrorState({ title = "Не удалось загрузить", detail, onRetry }) {
  return (
    <div className="state" role="alert">
      <span className="state-glyph">!</span>
      <h2>{title}</h2>
      {detail && <p>{detail}</p>}
      {onRetry && (
        <button type="button" className="btn primary" onClick={onRetry}>
          Повторить
        </button>
      )}
    </div>
  );
}

export function EmptyState({ title = "Пока нет данных", detail }) {
  return (
    <div className="empty">
      <strong>{title}</strong>
      {detail && <p>{detail}</p>}
    </div>
  );
}

export function Skeleton({ rows = 4 }) {
  return (
    <div className="skeleton-list" aria-hidden="true">
      {Array.from({ length: rows }, (unused, index) => (
        <i key={index} />
      ))}
    </div>
  );
}

export function SegmentedControl({ value, options, onChange, ariaLabel }) {
  return (
    <div className="segmented" role="group" aria-label={ariaLabel}>
      {options.map(([id, text]) => (
        <button
          type="button"
          key={id}
          className={String(value) === String(id) ? "active" : ""}
          aria-pressed={String(value) === String(id)}
          onClick={() => onChange(id)}
        >
          {text}
        </button>
      ))}
    </div>
  );
}

export function SearchInput({ value, onChange, placeholder = "Поиск..." }) {
  const [local, setLocal] = useState(value ?? "");
  useEffect(() => setLocal(value ?? ""), [value]);
  useEffect(() => {
    const timer = setTimeout(() => {
      if (local !== (value ?? "")) onChange(local);
    }, 300);
    return () => clearTimeout(timer);
  }, [local, onChange, value]);
  return (
    <label className="search">
      <span aria-hidden="true">⌕</span>
      <input value={local} placeholder={placeholder} onChange={(event) => setLocal(event.target.value)} aria-label={placeholder} />
      {local && (
        <button type="button" onClick={() => setLocal("")} aria-label="Очистить поиск">
          ×
        </button>
      )}
    </label>
  );
}

export function Select({ value, options, onChange, label, ariaLabel }) {
  return (
    <label className="field inline">
      {label && <span>{label}</span>}
      <select value={value ?? ""} onChange={(event) => onChange(event.target.value)} aria-label={ariaLabel || label}>
        {options.map(([id, text]) => (
          <option value={id} key={id}>
            {text}
          </option>
        ))}
      </select>
    </label>
  );
}

/**
 * Table with server-side pagination. `columns` entries describe the header, a
 * render function and an optional alignment/width so column layout stays declarative.
 */
export function DataTable({ columns, rows, keyOf, page, onPageChange, loading, emptyTitle, emptyDetail, onRowClick }) {
  const total = page?.total ?? rows.length;
  const limit = page?.limit ?? rows.length ?? 0;
  const offset = page?.offset ?? 0;
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + limit, total);
  const canPrev = offset > 0;
  const canNext = offset + limit < total;

  return (
    <div className="table-wrap">
      <table className="table">
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column.key} style={{ width: column.width, textAlign: column.align || "left" }}>
                {column.title}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr
              key={keyOf(row)}
              className={onRowClick ? "clickable" : ""}
              onClick={onRowClick ? () => onRowClick(row) : undefined}
              tabIndex={onRowClick ? 0 : undefined}
              onKeyDown={
                onRowClick
                  ? (event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        onRowClick(row);
                      }
                    }
                  : undefined
              }
            >
              {columns.map((column) => (
                <td key={column.key} style={{ textAlign: column.align || "left" }} data-label={column.title}>
                  {column.render(row)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {loading && !rows.length && <Skeleton rows={5} />}
      {!loading && !rows.length && <EmptyState title={emptyTitle} detail={emptyDetail} />}
      {onPageChange && total > 0 && (
        <div className="table-foot">
          <span>
            {from}–{to} из {total}
          </span>
          <div className="pager-buttons">
            <button type="button" className="btn ghost" disabled={!canPrev} onClick={() => onPageChange(Math.max(0, offset - limit))}>
              Назад
            </button>
            <button type="button" className="btn ghost" disabled={!canNext} onClick={() => onPageChange(offset + limit)}>
              Вперёд
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export function Field({ label, hint, children, wide }) {
  return (
    <label className={wide ? "field wide" : "field"}>
      <span>{label}</span>
      {children}
      {hint && <small>{hint}</small>}
    </label>
  );
}

export function Toggle({ checked, onChange, label }) {
  return (
    <label className="toggle">
      <input type="checkbox" checked={Boolean(checked)} onChange={(event) => onChange(event.target.checked)} />
      <i aria-hidden="true" />
      <span>{label}</span>
    </label>
  );
}

export function Modal({ title, onClose, children, footer }) {
  const ref = useRef(null);
  useEffect(() => {
    const handler = (event) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handler);
    ref.current?.querySelector("input, select, button")?.focus();
    return () => document.removeEventListener("keydown", handler);
  }, [onClose]);
  return (
    <div className="overlay" onMouseDown={onClose} role="presentation">
      <div
        className="modal"
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="modal-head">
          <h2>{title}</h2>
          <button type="button" className="btn icon" onClick={onClose} aria-label="Закрыть">
            ×
          </button>
        </div>
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-foot">{footer}</div>}
      </div>
    </div>
  );
}

export function Toast({ notice, onDismiss }) {
  useEffect(() => {
    if (!notice) return undefined;
    const timer = setTimeout(onDismiss, 4200);
    return () => clearTimeout(timer);
  }, [notice, onDismiss]);
  if (!notice) return null;
  return (
    <div className={`toast ${notice.tone || "ok"}`} role="status" aria-live="polite">
      <span>{notice.message}</span>
      <button type="button" onClick={onDismiss} aria-label="Скрыть уведомление">
        ×
      </button>
    </div>
  );
}

export function CommandPalette({ items, onSelect, onClose }) {
  const [query, setQuery] = useState("");
  const [focus, setFocus] = useState(0);
  const inputRef = useRef(null);
  useEffect(() => inputRef.current?.focus(), []);
  const filtered = useMemo(() => {
    if (!query.trim()) return items;
    const needle = query.trim().toLowerCase();
    return items.filter(([, title, , keywords]) =>
      `${title} ${keywords || ""}`.toLowerCase().includes(needle),
    );
  }, [items, query]);
  useEffect(() => setFocus(0), [query]);
  return (
    <div className="overlay top" onMouseDown={onClose} role="presentation">
      <div className="palette" onMouseDown={(event) => event.stopPropagation()} role="dialog" aria-modal="true" aria-label="Быстрый переход">
        <input
          ref={inputRef}
          value={query}
          placeholder="Раздел или действие..."
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setFocus((index) => Math.min(index + 1, filtered.length - 1));
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setFocus((index) => Math.max(index - 1, 0));
            }
            if (event.key === "Enter" && filtered[focus]) onSelect(filtered[focus][0]);
          }}
        />
        <div className="palette-list">
          {filtered.length ? (
            filtered.map(([id, title, icon], index) => (
              <button
                type="button"
                key={id}
                className={index === focus ? "focused" : ""}
                onMouseEnter={() => setFocus(index)}
                onClick={() => onSelect(id)}
              >
                <span className="palette-icon">{icon}</span>
                {title}
              </button>
            ))
          ) : (
            <p className="palette-empty">Ничего не найдено</p>
          )}
        </div>
      </div>
    </div>
  );
}
