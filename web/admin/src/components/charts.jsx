import { useId, useMemo, useState } from "react";

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

const niceCeiling = (value) => {
  if (value <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(value));
  const normalized = value / magnitude;
  const step = [1, 1.5, 2, 2.5, 3, 4, 5, 7.5, 10].find((candidate) => normalized <= candidate) ?? 10;
  return step * magnitude;
};

/** Builds a smooth cubic path so the area chart reads as a curve, not a polyline. */
const smoothPath = (points) => {
  if (points.length < 2) return points.length ? `M${points[0].x} ${points[0].y}` : "";
  const segments = [`M${points[0].x} ${points[0].y}`];
  for (let index = 0; index < points.length - 1; index += 1) {
    const current = points[index];
    const next = points[index + 1];
    const midX = (current.x + next.x) / 2;
    segments.push(`C${midX} ${current.y} ${midX} ${next.y} ${next.x} ${next.y}`);
  }
  return segments.join(" ");
};

function ChartFrame({ children, height = 220, className = "" }) {
  return (
    <div className={`chart-frame ${className}`} style={{ height }}>
      {children}
    </div>
  );
}

function Tooltip({ hover, formatValue, labelOf }) {
  if (!hover) return null;
  return (
    <div className="chart-tooltip" style={{ left: `${hover.left}%`, top: `${hover.top}%` }} role="presentation">
      <span className="ct-label">{labelOf(hover.point)}</span>
      {hover.series.map((entry) => (
        <span className="ct-row" key={entry.name}>
          <i style={{ background: entry.color }} />
          <span>{entry.name}</span>
          <b>{formatValue(entry.value)}</b>
        </span>
      ))}
    </div>
  );
}

/**
 * Multi-series area chart with a gradient fill, horizontal grid, y-axis ticks and
 * a hover crosshair. Series share one y-scale so they stay visually comparable.
 */
export function AreaChart({
  series,
  labels,
  height = 240,
  formatValue = (value) => value,
  formatTick = (value) => value,
  emptyLabel = "Нет данных за период",
}) {
  const gradientId = useId();
  const [hoverIndex, setHoverIndex] = useState(null);
  const width = 100;
  const chartHeight = 100;
  const padTop = 6;
  const padBottom = 14;

  const model = useMemo(() => {
    const lengths = series.map((entry) => entry.values.length);
    const length = Math.max(0, ...lengths);
    if (!length) return null;
    const peak = Math.max(...series.flatMap((entry) => entry.values.map(Number)), 0);
    const max = niceCeiling(peak || 1);
    const stepX = length > 1 ? width / (length - 1) : 0;
    const scaleY = (value) =>
      padTop + (chartHeight - padTop - padBottom) * (1 - clamp(Number(value) / max, 0, 1));
    return {
      length,
      max,
      ticks: [0, 0.25, 0.5, 0.75, 1].map((ratio) => ({ ratio, value: max * ratio })),
      plotted: series.map((entry) => ({
        ...entry,
        points: entry.values.map((value, index) => ({
          x: index * stepX,
          y: scaleY(value),
          value: Number(value),
          index,
        })),
      })),
    };
  }, [series]);

  if (!model) return <ChartFrame height={height}><p className="chart-empty">{emptyLabel}</p></ChartFrame>;

  const hover =
    hoverIndex == null
      ? null
      : {
          point: hoverIndex,
          left: clamp((hoverIndex / Math.max(1, model.length - 1)) * 100, 8, 92),
          top: 4,
          series: model.plotted.map((entry) => ({
            name: entry.name,
            color: entry.color,
            value: entry.values[hoverIndex] ?? 0,
          })),
        };

  return (
    <ChartFrame height={height}>
      <div className="chart-axis" aria-hidden="true">
        {[...model.ticks].reverse().map((tick) => (
          <span key={tick.ratio}>{formatTick(tick.value)}</span>
        ))}
      </div>
      <div className="chart-plot">
        <svg viewBox={`0 0 ${width} ${chartHeight}`} preserveAspectRatio="none" role="img" aria-label={series.map((entry) => entry.name).join(", ")}>
          <defs>
            {model.plotted.map((entry, index) => (
              <linearGradient id={`${gradientId}-${index}`} x1="0" y1="0" x2="0" y2="1" key={entry.name}>
                <stop offset="0%" stopColor={entry.color} stopOpacity="0.34" />
                <stop offset="100%" stopColor={entry.color} stopOpacity="0" />
              </linearGradient>
            ))}
          </defs>
          {model.ticks.map((tick) => {
            const y = padTop + (chartHeight - padTop - padBottom) * (1 - tick.ratio);
            return (
              <line
                key={tick.ratio}
                x1="0"
                x2={width}
                y1={y}
                y2={y}
                className={tick.ratio === 0 ? "grid-base" : "grid-line"}
              />
            );
          })}
          {model.plotted.map((entry, index) => (
            <g key={entry.name}>
              <path
                d={`${smoothPath(entry.points)} L${width} ${chartHeight - padBottom} L0 ${chartHeight - padBottom} Z`}
                fill={`url(#${gradientId}-${index})`}
              />
              <path d={smoothPath(entry.points)} fill="none" stroke={entry.color} strokeWidth="0.9" vectorEffect="non-scaling-stroke" />
            </g>
          ))}
          {hoverIndex != null && (
            <g>
              <line
                x1={model.plotted[0].points[hoverIndex]?.x ?? 0}
                x2={model.plotted[0].points[hoverIndex]?.x ?? 0}
                y1={padTop}
                y2={chartHeight - padBottom}
                className="grid-cursor"
              />
              {model.plotted.map((entry) => (
                <circle
                  key={entry.name}
                  cx={entry.points[hoverIndex]?.x ?? 0}
                  cy={entry.points[hoverIndex]?.y ?? 0}
                  r="1.6"
                  fill="var(--surface)"
                  stroke={entry.color}
                  strokeWidth="0.9"
                  vectorEffect="non-scaling-stroke"
                />
              ))}
            </g>
          )}
        </svg>
        <div className="chart-hit" onMouseLeave={() => setHoverIndex(null)}>
          {Array.from({ length: model.length }, (unused, index) => (
            <button
              type="button"
              key={index}
              aria-label={`${labels?.[index] ?? index}: ${model.plotted
                .map((entry) => `${entry.name} ${formatValue(entry.values[index] ?? 0)}`)
                .join(", ")}`}
              onMouseEnter={() => setHoverIndex(index)}
              onFocus={() => setHoverIndex(index)}
              onBlur={() => setHoverIndex(null)}
            />
          ))}
        </div>
        <Tooltip hover={hover} formatValue={formatValue} labelOf={(index) => labels?.[index] ?? ""} />
      </div>
      <div className="chart-labels" aria-hidden="true">
        {labels?.map((text, index) => (
          <span key={`${text}-${index}`} data-visible={model.length <= 14 || index % Math.ceil(model.length / 10) === 0}>
            {text}
          </span>
        ))}
      </div>
    </ChartFrame>
  );
}

/** Vertical bars for one series; used where discrete daily counts matter more than trend. */
export function BarChart({ values, labels, color = "var(--chart-1)", height = 200, formatValue = (value) => value, formatTick = (value) => value }) {
  const [hoverIndex, setHoverIndex] = useState(null);
  const max = niceCeiling(Math.max(...values.map(Number), 0) || 1);
  if (!values.length) return <ChartFrame height={height}><p className="chart-empty">Нет данных за период</p></ChartFrame>;
  return (
    <ChartFrame height={height}>
      <div className="chart-axis" aria-hidden="true">
        {[1, 0.75, 0.5, 0.25, 0].map((ratio) => (
          <span key={ratio}>{formatTick(max * ratio)}</span>
        ))}
      </div>
      <div className="chart-plot">
        <div className="bar-grid" aria-hidden="true">
          {[0, 1, 2, 3, 4].map((line) => (
            <i key={line} />
          ))}
        </div>
        <div className="bar-track" onMouseLeave={() => setHoverIndex(null)}>
          {values.map((value, index) => (
            <button
              type="button"
              key={index}
              className={hoverIndex === index ? "bar active" : "bar"}
              onMouseEnter={() => setHoverIndex(index)}
              onFocus={() => setHoverIndex(index)}
              onBlur={() => setHoverIndex(null)}
              aria-label={`${labels?.[index] ?? index}: ${formatValue(value)}`}
            >
              <i style={{ height: `${Math.max(2, (Number(value) / max) * 100)}%`, background: color }} />
            </button>
          ))}
        </div>
        {hoverIndex != null && (
          <div
            className="chart-tooltip"
            style={{ left: `${clamp(((hoverIndex + 0.5) / values.length) * 100, 10, 90)}%`, top: "4%" }}
          >
            <span className="ct-label">{labels?.[hoverIndex]}</span>
            <span className="ct-row">
              <i style={{ background: color }} />
              <b>{formatValue(values[hoverIndex])}</b>
            </span>
          </div>
        )}
      </div>
      <div className="chart-labels" aria-hidden="true">
        {labels?.map((text, index) => (
          <span key={`${text}-${index}`} data-visible={values.length <= 14 || index % Math.ceil(values.length / 10) === 0}>
            {text}
          </span>
        ))}
      </div>
    </ChartFrame>
  );
}

const CHART_COLORS = ["var(--chart-1)", "var(--chart-2)", "var(--chart-3)", "var(--chart-4)", "var(--chart-5)"];

export const chartColor = (index) => CHART_COLORS[index % CHART_COLORS.length];

/** Donut with an external legend; percentages are computed from the passed totals. */
export function DonutChart({ entries, formatValue = (value) => value, size = 168, emptyLabel = "Нет данных" }) {
  const total = entries.reduce((sum, entry) => sum + Number(entry.value), 0);
  if (!entries.length || total <= 0) return <p className="chart-empty">{emptyLabel}</p>;
  const radius = 42;
  const circumference = 2 * Math.PI * radius;
  let offset = 0;
  return (
    <div className="donut">
      <svg viewBox="0 0 100 100" width={size} height={size} role="img" aria-label="Круговая диаграмма распределения">
        {entries.map((entry, index) => {
          const ratio = Number(entry.value) / total;
          const dash = ratio * circumference;
          const segment = (
            <circle
              key={entry.key}
              cx="50"
              cy="50"
              r={radius}
              fill="none"
              stroke={chartColor(index)}
              strokeWidth="11"
              strokeDasharray={`${dash} ${circumference - dash}`}
              strokeDashoffset={-offset}
              transform="rotate(-90 50 50)"
              strokeLinecap="butt"
            />
          );
          offset += dash;
          return segment;
        })}
        <text x="50" y="47" className="donut-value">
          {formatValue(total)}
        </text>
        <text x="50" y="58" className="donut-caption">
          всего
        </text>
      </svg>
      <ul className="donut-legend">
        {entries.map((entry, index) => (
          <li key={entry.key}>
            <i style={{ background: chartColor(index) }} />
            <span>{entry.label}</span>
            <b>{Math.round((Number(entry.value) / total) * 100)}%</b>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Horizontal ranked bars, for breakdowns where the label needs full width. */
export function RankedBars({ entries, formatValue = (value) => value, emptyLabel = "Нет данных" }) {
  if (!entries.length) return <p className="chart-empty">{emptyLabel}</p>;
  const max = Math.max(...entries.map((entry) => Number(entry.value)), 1);
  return (
    <ul className="ranked-bars">
      {entries.map((entry, index) => (
        <li key={entry.key}>
          <div className="rb-head">
            <span>{entry.label}</span>
            <b>{formatValue(entry.value)}</b>
          </div>
          <div className="rb-track">
            <i style={{ width: `${Math.max(2, (Number(entry.value) / max) * 100)}%`, background: chartColor(index) }} />
          </div>
          {entry.caption && <small>{entry.caption}</small>}
        </li>
      ))}
    </ul>
  );
}

/** Compact trend line drawn inside metric cards. */
export function Sparkline({ values, color = "var(--chart-1)", height = 34 }) {
  const gradientId = useId();
  if (!values?.length) return <div className="sparkline" style={{ height }} />;
  const max = Math.max(...values.map(Number), 1);
  const stepX = values.length > 1 ? 100 / (values.length - 1) : 0;
  const points = values.map((value, index) => ({
    x: index * stepX,
    y: 28 - 26 * clamp(Number(value) / max, 0, 1),
  }));
  return (
    <svg className="sparkline" viewBox="0 0 100 30" preserveAspectRatio="none" style={{ height }} aria-hidden="true">
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.32" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={`${smoothPath(points)} L100 30 L0 30 Z`} fill={`url(#${gradientId})`} />
      <path d={smoothPath(points)} fill="none" stroke={color} strokeWidth="1.4" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}
