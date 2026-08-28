import http from "node:http";
import { chromium } from "../../web/node_modules/playwright/index.mjs";

const portArg = process.argv.indexOf("--port");
const port = portArg >= 0 ? Number(process.argv[portArg + 1]) : 9223;
const browser = await chromium.launch({ headless: true });

const server = http.createServer(async (request, response) => {
  if (request.method !== "POST" || request.url !== "/render") {
    response.writeHead(404).end();
    return;
  }
  try {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    const width = Number(input.width ?? 1280);
    const height = Number(input.height ?? 720);
    if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1 || width > 8192 || height > 8192) {
      throw new Error("invalid viewport dimensions");
    }
    const page = await browser.newPage({ viewport: { width, height } });
    await page.goto(String(input.url), { waitUntil: "networkidle", timeout: 30_000 });
    if (input.wait) await page.waitForFunction(String(input.wait), undefined, { timeout: 30_000 });
    const target = input.selector ? page.locator(String(input.selector)) : page;
    const png = await target.screenshot({ type: "png" });
    await page.close();
    response.writeHead(200, { "content-type": "image/png", "content-length": png.length }).end(png);
  } catch (error) {
    const body = JSON.stringify({ error: String(error) });
    response.writeHead(400, { "content-type": "application/json" }).end(body);
  }
});

server.listen(port, "0.0.0.0");
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, async () => {
    server.close();
    await browser.close();
    process.exit(0);
  });
}
