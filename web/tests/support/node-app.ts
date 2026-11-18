// What a browser gives the generated client that Node does not: a document origin for its
// relative /api paths. Imported first by every module that loads the client.

const origin: { current: string | undefined } = { current: undefined };

export const setOrigin = (url: string): void => {
  origin.current = url;
};

const nativeFetch = globalThis.fetch;

globalThis.fetch = async (input, init): Promise<Response> =>
  nativeFetch(
    typeof input === 'string' && input.startsWith('/') && origin.current !== undefined
      ? new URL(input, origin.current)
      : input,
    init,
  );
