// Prints the CV from a build of its own (CV_PRINT=1 adds /cv/<edition>/), tagged with its metadata by
// exiftool, then raised to PDF 2.0 by qpdf: the public edition to public/, and, given the overlay
// CV_RECRUITER names, the recruiter edition beside that overlay.
import { mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import process from 'node:process';

import { type Browser, chromium } from '@playwright/test';
import { run, runScript } from '@tonalisk-labs/kit/script';
import { build, preview } from 'astro';

import type { Edition } from '../src/lib/cv.ts';

const ROOT = path.resolve(import.meta.dirname, '..');

/** The roles both editions are written for, as a parser's keywords. */
const KEYWORDS = [
  'Programme Manager',
  'Delivery Director',
  'Head of Delivery',
  'Transformation Lead',
  'Operational Excellence',
  'Continuous Improvement',
  'Head of PMO',
  'Portfolio Management',
  'Delivery Governance',
  'DORA metrics',
  'Value Stream Mapping',
  'Lean Six Sigma Black Belt',
  'SAFe Scrum Master',
  'AI-assisted SDLC',
];

/** What the document says about itself, in its XMP packet and its information dictionary. */
interface Meta {
  readonly title: string;
  readonly author: string;
  /** One sentence: XMP's dc:description and the information dictionary's Subject. */
  readonly description: string;
  readonly keywords: readonly string[];
  /** BCP 47, as the catalog's /Lang says it. */
  readonly language: string;
}

const META: Meta = {
  title: 'Marco Ferra, CV',
  author: 'Marco Ferra',
  description:
    'CV of Marco Ferra, Programme Manager: delivery governance, transformation and operational excellence.',
  keywords: KEYWORDS,
  language: 'en-GB',
};

const OVERLAY = process.env['CV_RECRUITER'];

const PUBLIC_PDF = path.join(ROOT, 'public', 'Marco-Ferra-CV.pdf');

/** The recruiter edition goes beside its overlay, outside this repo. */
const recruiterPdf = (overlay: string): string =>
  path.join(path.dirname(path.resolve(overlay)), 'Marco-Ferra-CV-Full.pdf');

const OUTPUTS: ReadonlyMap<Edition, string> = new Map<Edition, string>(
  OVERLAY === undefined
    ? [['public', PUBLIC_PDF]]
    : [
        ['public', PUBLIC_PDF],
        ['recruiter', recruiterPdf(OVERLAY)],
      ],
);

const PRODUCER = 'Skia/PDF, raised to PDF 2.0 by qpdf';

/** The tags exiftool writes: the XMP packet PDF 2.0 reads, and the information dictionary for
 *  the parsers that read only that. */
const tagsOf = (meta: Meta): string[] => [
  '-XMP-dc:Format=application/pdf',
  `-XMP-dc:Title=${meta.title}`,
  `-XMP-dc:Creator=${meta.author}`,
  `-XMP-dc:Description=${meta.description}`,
  ...meta.keywords.map((keyword) => `-XMP-dc:Subject=${keyword}`),
  `-XMP-dc:Language=${meta.language}`,
  `-XMP-pdf:Keywords=${meta.keywords.join(', ')}`,
  `-XMP-pdf:Producer=${PRODUCER}`,
  '-XMP-xmp:CreatorTool=Chromium',
  ...['CreateDate', 'ModifyDate', 'MetadataDate'].map((date) => `-XMP-xmp:${date}=now`),
  `-PDF:Title=${meta.title}`,
  `-PDF:Author=${meta.author}`,
  `-PDF:Subject=${meta.description}`,
  `-PDF:Keywords=${meta.keywords.join(', ')}`,
  '-PDF:Creator=Chromium',
  `-PDF:Producer=${PRODUCER}`,
  '-PDF:CreateDate=now',
  '-PDF:ModifyDate=now',
];

/** Tags `printed` while it is still PDF 1.x, whose information dictionary exiftool writes, then
 *  raises it to PDF 2.0 at `output`: qpdf writes one revision, the same bytes for the same input,
 *  and checks it. Its dictionaries stay plain text, which the CV's privacy test reads. */
const finish = async (printed: string, output: string, stop: AbortSignal): Promise<void> => {
  const options = { signal: stop };
  await run('exiftool', ['-q', '-overwrite_original', ...tagsOf(META), printed], options);
  await run('qpdf', ['--deterministic-id', '--force-version=2.0', printed, output], options);
  await run('qpdf', ['--check', output], options);
};

/** One edition as Chromium prints it: print media from the start, so the font loads first. */
const print = async (browser: Browser, origin: string, edition: Edition): Promise<Buffer> => {
  const page = await browser.newPage();
  await page.emulateMedia({ media: 'print' });
  await page.goto(`${origin}/cv/${edition}/`, { waitUntil: 'networkidle' });
  await page.evaluate(async () => {
    await globalThis.document.fonts.ready;
  });
  return page.pdf({
    format: 'A4',
    tagged: true,
    outline: true,
    preferCSSPageSize: true,
    printBackground: true,
  });
};

const printAll = async (outDir: string, work: string, stop: AbortSignal): Promise<void> => {
  const server = await preview({
    root: ROOT,
    outDir,
    logLevel: 'warn',
    server: { host: '127.0.0.1', port: 4390 },
  });
  const browser = await chromium
    .launch({ handleSIGINT: false, handleSIGTERM: false, handleSIGHUP: false })
    .catch(async (error: unknown) => {
      await server.stop();
      throw error;
    });
  const release = async (): Promise<void> => {
    await Promise.all([browser.close(), server.stop()]);
  };
  const closing: { done: Promise<void> | undefined } = { done: undefined };
  /** Closes the browser and the server once, whichever comes first: the end or a signal. */
  const close = async (): Promise<void> => {
    closing.done ??= release();
    await closing.done;
  };
  stop.addEventListener('abort', () => {
    close().catch((error: unknown) => {
      console.error(error);
    });
  });
  try {
    const origin = `http://127.0.0.1:${server.port.toString()}`;
    for (const [edition, output] of OUTPUTS) {
      stop.throwIfAborted();
      const printed = path.join(work, `${edition}.pdf`);
      // oxlint-disable-next-line no-await-in-loop -- one page at a time keeps the output ordered.
      await writeFile(printed, await print(browser, origin, edition));
      // oxlint-disable-next-line no-await-in-loop
      await mkdir(path.dirname(output), { recursive: true });
      // oxlint-disable-next-line no-await-in-loop
      await finish(printed, output, stop);
      // oxlint-disable-next-line no-await-in-loop
      const { size } = await stat(output);
      console.log(`cv-pdf: wrote ${path.relative(ROOT, output)} (${size.toString()} bytes)`);
    }
  } finally {
    await close();
  }
};

const main = async (stop: AbortSignal): Promise<number> => {
  const work = await mkdtemp(path.join(tmpdir(), 'cv-pdf-'));
  const outDir = path.join(work, 'dist');
  process.env['CV_PRINT'] = '1';
  try {
    await build({ root: ROOT, outDir, logLevel: 'warn' });
    stop.throwIfAborted();
    await printAll(outDir, work, stop);
    return 0;
  } finally {
    await rm(work, { recursive: true, force: true });
  }
};

await runScript('cv-pdf', main);
