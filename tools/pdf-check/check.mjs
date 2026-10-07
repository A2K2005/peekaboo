// Independent check that annotations show in another renderer (pdf.js).
// Usage: node check.mjs <file.pdf> <rects.json>
// rects.json: [{ "page": 0, "name": "Ink", "rect": [left, top, right, bottom] }]
// with rect values in 0..1 from the page's top-left corner.
// Each page renders twice, with and without annotations. A rectangle passes
// when enough pixels inside it differ. Prints JSON; exits 1 if any fails.
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { AnnotationMode, getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";

const SCALE = 2;
const MIN_CHANGED = 20;

const [pdfPath, rectsPath] = process.argv.slice(2);
if (!pdfPath || !rectsPath) {
  console.error("Usage: node check.mjs <file.pdf> <rects.json>");
  process.exit(2);
}
const require = createRequire(import.meta.url);
// pdf.js wants a trailing "/"; Node reads the files by path.
const fonts = path
  .join(path.dirname(require.resolve("pdfjs-dist/package.json")), "standard_fonts")
  .replaceAll("\\", "/") + "/";
const rects = JSON.parse(await readFile(rectsPath, "utf8"));
const data = new Uint8Array(await readFile(pdfPath));
const doc = await getDocument({ data, standardFontDataUrl: fonts }).promise;

async function render(page, annotationMode) {
  const viewport = page.getViewport({ scale: SCALE });
  const { canvas, context } = doc.canvasFactory.create(
    Math.ceil(viewport.width),
    Math.ceil(viewport.height),
  );
  await page.render({ canvas, canvasContext: context, viewport, annotationMode }).promise;
  return { width: canvas.width, height: canvas.height, pixels: context.getImageData(0, 0, canvas.width, canvas.height).data };
}

const results = [];
const pages = new Map();
for (const item of rects) {
  if (!pages.has(item.page)) {
    const page = await doc.getPage(item.page + 1);
    pages.set(item.page, {
      plain: await render(page, AnnotationMode.DISABLE),
      marked: await render(page, AnnotationMode.ENABLE),
    });
  }
  const { plain, marked } = pages.get(item.page);
  const [l, t, r, b] = item.rect;
  const x0 = Math.max(0, Math.floor(l * plain.width) - 2);
  const x1 = Math.min(plain.width, Math.ceil(r * plain.width) + 2);
  const y0 = Math.max(0, Math.floor(t * plain.height) - 2);
  const y1 = Math.min(plain.height, Math.ceil(b * plain.height) + 2);
  let changed = 0;
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const i = (y * plain.width + x) * 4;
      const diff = Math.abs(plain.pixels[i] - marked.pixels[i]) +
        Math.abs(plain.pixels[i + 1] - marked.pixels[i + 1]) +
        Math.abs(plain.pixels[i + 2] - marked.pixels[i + 2]);
      if (diff > 24) changed++;
    }
  }
  results.push({ name: item.name, page: item.page, changed, pass: changed >= MIN_CHANGED });
}
console.log(JSON.stringify(results));
process.exit(results.every((r) => r.pass) ? 0 : 1);
