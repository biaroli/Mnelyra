import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import packageJson from "./package.json";

const host = process.env.TAURI_DEV_HOST;
export default defineConfig(async () => ({
  plugins: [tailwindcss(), sveltekit()],

  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
  },

  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1422,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
