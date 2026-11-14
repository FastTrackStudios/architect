import { architect } from '../../webview.js';

const report = {};
const attempt = async (name, f) => {
  try { report[name] = { ok: await f() }; } catch (e) { report[name] = { error: e.kind ?? e.name ?? String(e) }; }
};

await attempt('own action', () => architect.run('DEMO_BOARD_PING'));
await attempt('outside action', () => architect.run('THEME_SET', { id: 'evil' }));
await attempt('direct fetch', () => fetch('/package.json').then((r) => r.status));
await attempt('host fetch', () => architect.fetch('https://api.example.com/x').then((r) => r.status));
await attempt('cookie', () => document.cookie);
await attempt('storage', () => localStorage.getItem('x'));
await attempt('parent dom', () => window.parent.document.title);
await attempt('top navigation', () => { window.top.location.href = 'https://evil.example/'; return 'navigated'; });
if (!document.getElementById('architect-theme')) await new Promise((ok) => architect.theme.onChange(ok));
report.brand = getComputedStyle(document.documentElement).getPropertyValue('--brand').trim();
report.scheme = architect.theme.scheme;
report.components = getComputedStyle(document.getElementById('root')).borderTopStyle;
const heard = new Promise((ok) => architect.on('doc.*', (detail, name) => ok({ name, detail })));
await architect.log('listening');
report.heard = await heard;
document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
await architect.log(JSON.stringify(report));
