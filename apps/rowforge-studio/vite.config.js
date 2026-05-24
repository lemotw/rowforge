import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
const port = 1420;
export default defineConfig(async () => ({
    plugins: [react()],
    resolve: {
        alias: {
            "@": path.resolve(__dirname, "./src"),
        },
    },
    clearScreen: false,
    server: {
        port,
        strictPort: true,
        host: process.env.TAURI_DEV_HOST || false,
        hmr: process.env.TAURI_DEV_HOST
            ? { protocol: "ws", host: process.env.TAURI_DEV_HOST, port: port + 1 }
            : undefined,
        watch: { ignored: ["**/src-tauri/**"] },
    },
}));
