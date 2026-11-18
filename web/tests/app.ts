// The backend needs mail, Google Calendar and Stripe's one-time payments (no plans), and no
// database.
import { randomBytes } from 'node:crypto';

import { defineAdapter, type Started } from '@tonalisk-labs/kit/testing';

import { employerNamed } from './support/employer.ts';

/** What the tests wait out before they send a form (support/api.ts). */
export const MIN_FILL_SECONDS = 1;

const required = <Service>(service: Service | undefined, name: string): Service => {
  if (service === undefined) {
    throw new Error(`the stack started no ${name}`);
  }
  return service;
};

/** The committed config, overridden where the stack differs. */
const backendEnv = ({
  origin,
  bindAddr,
  mail,
  google,
  stripe,
}: Started): Record<string, string> => {
  const { url } = required(google, 'fake Google Calendar');
  const relay = required(mail, 'mail relay');
  const payments = required(stripe, 'Stripe');
  return {
    BIND_ADDR: bindAddr,
    PUBLIC_BASE_URL: origin,
    // The harness edge names the client as a forwarded address, not as CloudFront does.
    CLIENT_IP_HEADER: 'x-forwarded-for',
    MIN_FILL_SECONDS: String(MIN_FILL_SECONDS),
    // Every row books; the cap is for production's relay, not the suite's.
    DAILY_SEND_CAP: '1000000',
    SERVER_SECRET: randomBytes(32).toString('hex'),
    GOOGLE_API_BASE: url,
    GOOGLE_TOKEN_URL: `${url}/token`,
    GOOGLE_CLIENT_ID: 'journeys',
    GOOGLE_CLIENT_SECRET: 'journeys',
    GOOGLE_REFRESH_TOKEN: 'journeys',
    SMTP_HOST: relay.host,
    SMTP_PORT: String(relay.port),
    SMTP_USERNAME: 'journeys',
    SMTP_PASSWORD: 'journeys',
    STRIPE_API_BASE: payments.url,
    STRIPE_SECRET_KEY: payments.secretKey,
    STRIPE_WEBHOOK_SECRET: payments.webhookSecret,
    // Read by rustls-native-certs: the backend trusts the stack's throwaway CA alone.
    SSL_CERT_FILE: relay.caFile,
  };
};

export const adapter = defineAdapter({
  env: {},
  backend: {
    crate: 'marcoferra-server',
    apiPrefix: '/api',
    roles: [],
    mail: true,
    stripe: { plans: [], webhookPath: '/api/stripe/webhook' },
    google: true,
    env: backendEnv,
  },
  // Every built page as the browser holds it: text, attributes, metadata and JSON-LD.
  pageChecks: async (page) => employerNamed(await page.content()),
  // Cancelling and a paid booking's confirmation are reached from a link alone (astro.config.mjs).
  seo: { noindex: ['/book/cancel/', '/book/*/done/'], disallow: [] },
});
