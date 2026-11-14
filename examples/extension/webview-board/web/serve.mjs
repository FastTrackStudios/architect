// Serves the repository for web/index.html: the package's files under the
// web view policy (WEBVIEW_CSP) and readable from the frame's opaque origin
// (CORS), as a node serves a web view's package. `node serve.mjs [port]`.
import { createServer } from 'node:http';
import { readFileSync, statSync } from 'node:fs';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import { WEBVIEW_CSP } from '../../../../features/extensions/web/webview-host.js';

const root = normalize(fileURLToPath(new URL('../../../../', import.meta.url)));
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.json': 'application/json' };
const port = Number(process.argv[2] ?? 4317);
createServer((req, res) => {
  let path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  if (path.endsWith('/')) path += 'index.html';
  const file = normalize(join(root, path));
  if (!file.startsWith(root)) return res.writeHead(403).end();
  try {
    if (!statSync(file).isFile()) throw new Error('not a file');
    const headers = { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-store' };
    if (path.includes('/package/')) {
      headers['access-control-allow-origin'] = '*';
      if (extname(file) === '.html') headers['content-security-policy'] = WEBVIEW_CSP;
    }
    res.writeHead(200, headers).end(readFileSync(file));
  } catch {
    res.writeHead(404).end('not found');
  }
}).listen(port, '127.0.0.1', () => console.log(`http://127.0.0.1:${port}/examples/extension/webview-board/web/`));
