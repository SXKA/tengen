import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Cross-origin isolation enables SharedArrayBuffer, which the multi-threaded
// engine needs. Production sets the same headers in vercel.json.
const isolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  plugins: [react()],
  worker: { format: "es" },
  build: { target: "es2022" },
  server: { headers: isolation },
  preview: { headers: isolation },
});
