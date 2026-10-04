// v2.23.2.6 - Serve synthetic offline formula fixtures on the persistent project port.
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const projectPath = path.resolve(import.meta.dirname, '..');
const fixturePath = path.join(projectPath, 'release_artifacts', 'verification', 'v2.23.2.6', 'math_preview');
const allocatorPath = path.join(process.env.USERPROFILE, '.local-web', 'project-port.mjs');
const { listenWithPersistentProjectPort } = await import(pathToFileURL(allocatorPath).href);
const identity = 'tenfold_android_math_v2.23.2.6';
const server = http.createServer(async (request, response) => {
  if (request.url === '/__local_web_identity') {
    response.writeHead(200, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ identity, projectPath }));
    return;
  }
  const fileName = request.url === '/dark.html' ? 'dark.html' :
    request.url === '/' || request.url === '/light.html' ? 'light.html' : null;
  if (!fileName) { response.writeHead(404); response.end(); return; }
  try {
    const bytes = await readFile(path.join(fixturePath, fileName));
    response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' });
    response.end(bytes);
  } catch { response.writeHead(503); response.end('Fixture unavailable'); }
});
const allocation = await listenWithPersistentProjectPort(server, { projectPath, role: 'public' });
process.stdout.write(JSON.stringify({ identity, ...allocation }) + '\n');
