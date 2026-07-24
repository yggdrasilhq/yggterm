import { fileURLToPath, URL } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Builds ONE self-contained asset pair (JS + CSS) that yggterm serves into a
// web surface. Deliberately not upstream's config: no tanstack router plugin
// (we have no routes), no react-compiler babel pass (a build-speed/behaviour
// risk we do not need), no dev WS URL define (we have no server).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: [
      // Upstream's two import styles, pointed at the vendored tree.
      { find: "@t3tools/contracts", replacement: fileURLToPath(new URL("./src/vendor/contracts.ts", import.meta.url)) },
      { find: /^~\//, replacement: fileURLToPath(new URL("./src/vendor/", import.meta.url)) },
      // One vendored consumer reads a route param; a yggterm surface is
      // single-session, so the router is a stub rather than a dependency.
      { find: "@tanstack/react-router", replacement: fileURLToPath(new URL("./src/vendor/router-shim.ts", import.meta.url)) },
    ],
  },
  // A `lib` build skips vite's app-mode env replacement, so React's
  // `process.env.NODE_ENV` checks survive into the browser bundle and throw
  // "process is not defined" before mount() is ever reached. Replacing it here
  // also drops React's dev-only branches from the output.
  define: {
    "process.env.NODE_ENV": JSON.stringify("production"),
    "process.env": "{}",
  },
  build: {
    // A library build with an inlined CSS import would hand yggterm a JS blob
    // that injects styles at runtime; emitting a real .css file instead lets
    // the surface show a styled first paint rather than a flash of unstyled
    // transcript.
    lib: {
      entry: fileURLToPath(new URL("./src/mount.tsx", import.meta.url)),
      name: "yggtermTranscript",
      formats: ["iife"],
      fileName: () => "transcript.js",
    },
    outDir: "dist",
    emptyOutDir: true,
    cssCodeSplit: false,
    sourcemap: false,
  },
});
