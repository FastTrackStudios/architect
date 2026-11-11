import { defineConfig } from 'vite';

// Nothing special: the package starts its workers from a string (no
// worker files for Vite to find), and the extension is static files in
// public/counter (`npm run extension`).
export default defineConfig({
  build: { target: 'es2022' },
});
