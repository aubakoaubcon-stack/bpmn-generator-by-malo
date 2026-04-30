import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    // Web runs on 5174; API runs on 5175 (Express).
    // Proxy keeps frontend code environment-agnostic: fetch('/api/...').
    proxy: {
      "/api": {
        target: process.env.VITE_API_TARGET || "http://localhost:5175",
        changeOrigin: true,
      },
    },
  },
});

