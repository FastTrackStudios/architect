// A ```showcase-chart fence as an SVG bar chart: `base:` and `by:` lines
// in the fence, the counts from Chart's own action, each bar in the
// theme's chart colours.
import { h, useEmbed, useData, useTheme } from "architect";

type Bars = { by: string; bars: { label: string; count: number }[] };

function field(source: string, key: string): string {
  const m = source.match(new RegExp(`^\\s*${key}\\s*:\\s*(.+?)\\s*$`, "m"));
  return m ? m[1] : "";
}

export default function Chart() {
  const source = useEmbed()?.source ?? "";
  const theme = useTheme();
  const data = useData<Bars>("SHOWCASE_CHART_COUNT", { base: field(source, "base"), by: field(source, "by") });
  if (data.error) return <div style={{ color: "$destructive", padding: 8 }}>{data.error}</div>;
  const bars = data.value?.bars ?? [];
  const max = Math.max(1, ...bars.map((b) => b.count));
  const w = 320, rowH = 28, labelW = 110, gap = 6;
  const h2 = bars.length * (rowH + gap) + gap;
  // An SVG fill takes a colour, not a token: the theme's resolved values.
  const colours = [1, 2, 3, 4, 5].map((n) => theme.value(`chart-${n}`) ?? "#888");
  const ink = theme.value("foreground") ?? "currentColor";
  const muted = theme.value("muted-foreground") ?? ink;
  const font = "Inter, sans-serif";
  return (
    <div class="showcase-chart" style={{ padding: 8 }}>
      <div style={{ color: "$muted-foreground", fontSize: 12, paddingBottom: 4 }}>
        By {data.value?.by ?? "…"} ({theme.scheme})
      </div>
      {/* The fence's width, whatever it is (a phone's too): the drawing
          scales from its viewBox. */}
      <svg width="100%" viewBox={`0 0 ${w} ${h2}`} style={{ maxWidth: 480, display: "block" }} role="img">
        {bars.map((b, i) => {
          const y = gap + i * (rowH + gap);
          const len = ((w - labelW - 40) * b.count) / max;
          return (
            <g key={b.label}>
              <text x={0} y={y + rowH / 2 + 5} style={{ fill: ink, fontSize: 13, fontFamily: font }}>{b.label}</text>
              <rect x={labelW} y={y} width={len} height={rowH} rx={4} style={{ fill: colours[i % colours.length] }} />
              <text x={labelW + len + 6} y={y + rowH / 2 + 5} style={{ fill: muted, fontSize: 13, fontFamily: font }}>{String(b.count)}</text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}
