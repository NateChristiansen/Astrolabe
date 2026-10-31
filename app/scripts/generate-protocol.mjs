import { compileFromFile } from "json-schema-to-typescript";
import { mkdir, readdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const schemas = new URL("../../protocol/schemas/", import.meta.url);
const output = new URL("../src/protocol/", import.meta.url);
await mkdir(output, { recursive: true });
for (const file of (await readdir(schemas)).filter((f) => f.endsWith(".json")).sort()) {
  const types = await compileFromFile(fileURLToPath(new URL(file, schemas)), {
    bannerComment: "/* Generated from protocol/schemas. Do not edit. */",
    // Map schemars' draft 2020-12 open objects to TypeScript index signatures.
    additionalProperties: true,
  });
  await writeFile(new URL(file.replace(/\.json$/, ".ts"), output), types);
}
