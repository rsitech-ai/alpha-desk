import path from "node:path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig, loadEnv } from "vite"

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "")
  const hlApi = env.VITE_HL_API_ORIGIN || "http://127.0.0.1:8788"
  const proxy = {
    "/healthz": hlApi,
    "/readyz": hlApi,
    "/v1": hlApi,
  }

  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        "@": path.resolve(import.meta.dirname, "./src"),
      },
    },
    server: {
      host: "127.0.0.1",
      port: 5174,
      proxy,
    },
    preview: {
      host: "127.0.0.1",
      port: 4174,
      proxy,
    },
  }
})
