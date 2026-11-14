import { defineConfig } from 'vite';

// Built into the extension's package (`src = "dist/index.html"`), every
// path relative, so the files load from the package over the host's
// protocol (a desk) or the node (a frame).
export default defineConfig({
  base: './',
  build: {
    outDir: '../package/dist',
    emptyOutDir: true,
    // One script and one sheet: no inline scripts (the view's policy has
    // no 'unsafe-inline' for scripts), no module preload polyfill.
    modulePreload: false,
    assetsInlineLimit: 0,
    sourcemap: false,
  },
});
