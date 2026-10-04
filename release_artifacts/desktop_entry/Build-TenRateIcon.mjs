// v1.0.2 - Validate the SVG canvas before native-size rasterization, including 1024px previews.
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { readFile, writeFile } from "node:fs/promises";

const ICON_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
const currentDirectory = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);

function loadSharp() {
  const candidates = [
    "sharp",
    process.env.TENRATE_NODE_MODULES
      ? join(process.env.TENRATE_NODE_MODULES, "sharp")
      : null,
    join(
      homedir(),
      ".cache",
      "codex-runtimes",
      "codex-primary-runtime",
      "dependencies",
      "node",
      "node_modules",
      "sharp",
    ),
  ].filter(Boolean);

  for (const candidate of candidates) {
    try {
      return require(candidate);
    } catch (error) {
      if (error?.code !== "MODULE_NOT_FOUND") {
        throw error;
      }
    }
  }

  throw new Error(
    "The sharp renderer is unavailable. Install sharp or set TENRATE_NODE_MODULES to a node_modules directory that contains it.",
  );
}

function readPngSize(png) {
  const signature = "89504e470d0a1a0a";
  if (png.subarray(0, 8).toString("hex") !== signature) {
    throw new Error("Rendered icon frame is not a PNG image.");
  }
  return {
    width: png.readUInt32BE(16),
    height: png.readUInt32BE(20),
  };
}

function svgAtSize(source, size) {
  const canvasDeclaration = '<svg width="1024" height="1024"';
  if (!source.includes(canvasDeclaration)) {
    throw new Error("The SVG master must declare a 1024x1024 root canvas.");
  }
  return Buffer.from(source.replace(
    canvasDeclaration,
    `<svg width="${size}" height="${size}"`,
  ));
}

function buildIco(frames) {
  const directorySize = 6 + frames.length * 16;
  const directory = Buffer.alloc(directorySize);
  directory.writeUInt16LE(0, 0);
  directory.writeUInt16LE(1, 2);
  directory.writeUInt16LE(frames.length, 4);

  let imageOffset = directorySize;
  frames.forEach(({ size, png }, index) => {
    const entryOffset = 6 + index * 16;
    directory.writeUInt8(size === 256 ? 0 : size, entryOffset);
    directory.writeUInt8(size === 256 ? 0 : size, entryOffset + 1);
    directory.writeUInt8(0, entryOffset + 2);
    directory.writeUInt8(0, entryOffset + 3);
    directory.writeUInt16LE(1, entryOffset + 4);
    directory.writeUInt16LE(32, entryOffset + 6);
    directory.writeUInt32LE(png.length, entryOffset + 8);
    directory.writeUInt32LE(imageOffset, entryOffset + 12);
    imageOffset += png.length;
  });

  return Buffer.concat([directory, ...frames.map(({ png }) => png)]);
}

const sharp = loadSharp();
const sourcePath = join(currentDirectory, "TenRate_Icon.svg");
const previewPath = join(currentDirectory, "TenRate_Icon.png");
const icoPath = join(currentDirectory, "TenRate_Icon.ico");
const source = await readFile(sourcePath, "utf8");

const frames = [];
for (const size of ICON_SIZES) {
  const png = await sharp(svgAtSize(source, size))
    .png({ compressionLevel: 9, adaptiveFiltering: true })
    .toBuffer();
  const dimensions = readPngSize(png);
  if (dimensions.width !== size || dimensions.height !== size) {
    throw new Error(
      `Invalid ${size}px frame dimensions: ${dimensions.width}x${dimensions.height}.`,
    );
  }
  frames.push({ size, png });
}

const preview = await sharp(svgAtSize(source, 1024))
  .png({ compressionLevel: 9, adaptiveFiltering: true })
  .toBuffer();
await writeFile(previewPath, preview);
await writeFile(icoPath, buildIco(frames));

console.log(`Wrote ${icoPath}`);
console.log(`Frames: ${ICON_SIZES.join(", ")}`);
