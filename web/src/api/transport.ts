// The one way a request leaves the page: orval routes every operation here. A POST travels
// with its body's SHA-256 (an empty one's too), which CloudFront needs to sign it for the Lambda.

interface Reply {
  status: number;
  data: unknown;
  headers: Headers;
}

/** Why a request has no answer from the API: no network, or something between that is not it. */
export type Fault = 'offline' | 'unavailable';

export class TransportError extends Error {
  public readonly fault: Fault;

  public constructor(fault: Fault) {
    super(fault);
    this.name = 'TransportError';
    this.fault = fault;
  }
}

/** A fetch or a body read the network cut off. */
const offline = (error: unknown): never => {
  throw error instanceof TypeError ? new TransportError('offline') : error;
};

/** The API answers a success as JSON and every refusal as problem JSON (RFC 9457). */
const JSON_TYPE = /^application\/json(?:;|$)/u;
const PROBLEM_TYPE = /^application\/problem\+json(?:;|$)/u;

const sha256 = async (body: string): Promise<string> => {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(body));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
};

/** A body the API did not write (a proxy's error page, a captive portal) is a fault, not data. */
const faultOf = (response: Response, body: string): Fault | undefined => {
  const type = response.status >= 400 ? PROBLEM_TYPE : JSON_TYPE;
  if (type.test(response.headers.get('content-type') ?? '') || (body === '' && response.ok)) {
    return undefined;
  }
  return navigator.onLine ? 'unavailable' : 'offline';
};

export const replyOf = async <Declared extends Reply>(response: Response): Promise<Declared> => {
  const body = await response.text().catch(offline);
  const fault = faultOf(response, body);
  if (fault !== undefined) {
    throw new TransportError(fault);
  }
  // The single trust boundary: the generated types are the contract for what a status carries.
  const described = (candidate: Reply): candidate is Declared =>
    candidate.status === response.status;
  const reply: Reply = {
    status: response.status,
    data: body === '' ? body : JSON.parse(body),
    headers: response.headers,
  };
  if (!described(reply)) {
    throw new TransportError('unavailable');
  }
  return reply;
};

/** Resolves to orval's `{ status, data, headers }`; a fetch the network refuses is `offline`. */
export const transport = async <Declared extends Reply>(
  url: string,
  options: RequestInit,
): Promise<Declared> => {
  const headers = new Headers(options.headers);
  if (typeof options.body === 'string' || options.method === 'POST') {
    headers.set(
      'x-amz-content-sha256',
      await sha256(typeof options.body === 'string' ? options.body : ''),
    );
  }
  const response = await fetch(url, Object.assign(structuredClone({}), options, { headers })).catch(
    offline,
  );
  return replyOf<Declared>(response);
};
