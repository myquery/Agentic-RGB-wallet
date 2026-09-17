import {defineConfig} from 'vitest/config';
import react from '@vitejs/plugin-react';
export default defineConfig({
  plugins:[react()],
  envDir: './.frontend-env-unused',
  server:{strictPort:true,port:5173,proxy:{'/api':{target:'http://127.0.0.1:3030',changeOrigin:true}}},
  test:{environment:'jsdom',setupFiles:'./src/test-setup.ts',exclude:['node_modules/**','e2e/**']}
});
