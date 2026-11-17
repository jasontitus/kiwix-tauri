import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
export default defineConfig({ plugins: [react()], envPrefix: ['VITE_', 'TAURI_ENV_PLATFORM'], clearScreen: false, server: { port: 1420, strictPort: true }, build: { target: 'es2022', rollupOptions: { input: { main: 'index.html', camera: 'camera.html' }, output: { charset: 'ascii' } } } });
