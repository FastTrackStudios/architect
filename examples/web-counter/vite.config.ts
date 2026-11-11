import { defineConfig } from 'vite';
import { SANDBOX_HASH } from '@fasttrackstudio/architect';

// The page's CSP names the extension sandbox's script by its hash
// (index.html, %SANDBOX_HASH%): the package's, whatever its version.
// Nothing else is special: the package starts its workers itself, and the
// extension is static files in public/counter (`npm run extension`).
export default defineConfig({
  build: { target: 'es2022' },
  plugins: [{ name: 'sandbox-hash', transformIndexHtml: (html) => html.replaceAll('%SANDBOX_HASH%', SANDBOX_HASH) }],
});
