// The web view showcase (#222): a Vite + TypeScript app using npm
// libraries (uPlot for the chart, SortableJS for the board), in an
// Architect pane. Everything it keeps is the extension's data; everything
// it asks of the app goes through the bridge.
import { requireArchitect, type Json } from '@fasttrackstudio/architect/webview';
import Sortable from 'sortablejs';
import uPlot from 'uplot';
import 'uplot/dist/uPlot.min.css';
import './style.css';

interface Card {
  id: string;
  title: string;
  points: number;
}

const architect = requireArchitect();
const KEY = 'cards';
const DEFAULT: Card[] = [
  { id: 'a', title: 'Chart the setlist', points: 3 },
  { id: 'b', title: 'Drag me up', points: 5 },
  { id: 'c', title: 'Theme follows the app', points: 2 },
  { id: 'd', title: 'Notify through the bridge', points: 1 },
];

const root = document.getElementById('app')!;
root.innerHTML = `
  <header class="arch-row" style="justify-content: space-between">
    <div>
      <h1 style="margin: 0; font-size: 18px">Board</h1>
      <div class="arch-muted" id="status">A web view app: Vite, TypeScript, uPlot and SortableJS.</div>
    </div>
    <div class="arch-row">
      <span class="arch-badge" id="scheme"></span>
      <button class="arch-button" id="add">Add a card</button>
      <button class="arch-button primary" id="notify">Notify</button>
    </div>
  </header>
  <section class="arch-card"><div id="chart"></div></section>
  <section class="arch-card" style="padding: 0"><ul class="arch-list" id="cards"></ul></section>
`;

const list = document.getElementById('cards') as HTMLUListElement;
const chartBox = document.getElementById('chart') as HTMLDivElement;
const status = document.getElementById('status')!;
let cards: Card[] = DEFAULT;
let chart: uPlot | null = null;

function token(name: string, fallback: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

function drawList() {
  list.replaceChildren(
    ...cards.map((c) => {
      const li = document.createElement('li');
      li.dataset.id = c.id;
      li.innerHTML = `<span class="handle" aria-hidden="true">⠿</span><span class="title"></span><span class="arch-badge"></span>`;
      li.querySelector('.title')!.textContent = c.title;
      li.querySelector('.arch-badge')!.textContent = `${c.points} pt`;
      return li;
    }),
  );
}

function drawChart() {
  const xs = cards.map((_, i) => i + 1);
  let total = 0;
  const running = cards.map((c) => (total += c.points));
  const width = Math.max(200, chartBox.clientWidth);
  const opts: uPlot.Options = {
    width,
    height: 160,
    legend: { show: false },
    cursor: { show: false },
    scales: { x: { time: false } },
    axes: [
      { stroke: token('--arch-muted', '#888'), grid: { stroke: token('--arch-line', '#8884') }, incrs: [1, 2, 5, 10] },
      { stroke: token('--arch-muted', '#888'), grid: { stroke: token('--arch-line', '#8884') } },
    ],
    series: [
      {},
      { label: 'points', stroke: token('--arch-brand', '#6c5ce7'), width: 2, fill: 'transparent', points: { size: 6 } },
    ],
  };
  chart?.destroy();
  chart = new uPlot(opts, [xs, running], chartBox);
}

async function save() {
  try {
    await architect.setData(KEY, cards as unknown as Json);
    status.textContent = `Saved ${cards.length} cards as the extension's data.`;
  } catch (e) {
    status.textContent = `Not saved: ${(e as Error).message}`;
  }
}

Sortable.create(list, {
  animation: 150,
  handle: '.handle',
  // Pointer events, not the HTML5 drag-and-drop API: the same on a mouse,
  // a pen and a finger, in every engine a web view is.
  forceFallback: true,
  onEnd() {
    const order = [...list.children].map((li) => (li as HTMLElement).dataset.id);
    cards = order.map((id) => cards.find((c) => c.id === id)!).filter(Boolean);
    drawChart();
    void save();
  },
});

document.getElementById('add')!.addEventListener('click', () => {
  const n = cards.length + 1;
  cards = [...cards, { id: `n${Date.now()}`, title: `Card ${n}`, points: (n % 5) + 1 }];
  drawList();
  drawChart();
  void save();
});

document.getElementById('notify')!.addEventListener('click', async () => {
  try {
    await architect.notify(`The board has ${cards.length} cards`, { title: 'Web view board' });
  } catch (e) {
    status.textContent = `Refused: ${(e as Error).message}`;
  }
});

function showScheme() {
  document.getElementById('scheme')!.textContent = architect.theme.scheme;
  drawChart();
}
architect.theme.onChange(showScheme);
new ResizeObserver(() => drawChart()).observe(chartBox);

const kept = await architect.data<Card[]>(KEY).catch(() => null);
if (Array.isArray(kept) && kept.length) cards = kept;
drawList();
showScheme();
