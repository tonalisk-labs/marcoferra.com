// The API client, generated from the server's contract: every operation goes through the site's
// transport. It's built, not committed, so it always matches openapi.yaml.
import { defineConfig } from 'orval';

export default defineConfig({
  marcoferra: {
    input: '../server/openapi.yaml',
    output: {
      target: 'src/api/endpoints.ts',
      client: 'fetch',
      mode: 'single',
      baseUrl: '/api',
      // The site's own compiler options decide the import style (`./transport.ts`).
      tsconfig: 'tsconfig.json',
      override: { mutator: { path: 'src/api/transport.ts', name: 'transport' } },
    },
  },
});
