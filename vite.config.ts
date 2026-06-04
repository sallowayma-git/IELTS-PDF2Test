import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

function devSourceParserPlugin(): Plugin {
  return {
    name: "ielts-author-studio-dev-source-parser",
    configureServer(server) {
      server.middlewares.use("/__dev_parse_source", async (req, res) => {
        if (req.method !== "POST") {
          res.statusCode = 405;
          res.end("method_not_allowed");
          return;
        }
        try {
          const chunks: Buffer[] = [];
          for await (const chunk of req) {
            chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
          }
          const payload = JSON.parse(Buffer.concat(chunks).toString("utf8")) as {
            name?: string;
            jobId?: string;
            mode?: string;
            contentBase64?: string;
            sourcePath?: string;
          };
          if (!payload.name || (!payload.contentBase64 && !payload.sourcePath)) {
            res.statusCode = 400;
            res.end(JSON.stringify({ error: "missing_name_or_source" }));
            return;
          }
          const tempDir = await mkdtemp(join(tmpdir(), "ielts-author-studio-dev-"));
          const inputPath = payload.contentBase64
            ? join(tempDir, `source${extname(payload.name).toLowerCase() || ".bin"}`)
            : payload.sourcePath || "";
          const outputPath = join(tempDir, "document-ir.json");
          if (payload.contentBase64) await writeFile(inputPath, Buffer.from(payload.contentBase64, "base64"));
          const parserPath = resolve("sidecars/python-parser/parser.py");
          await new Promise<void>((resolvePromise, reject) => {
            const child = spawn("python3", [
              parserPath,
              "parse",
              "--input",
              inputPath,
              "--output",
              outputPath,
              "--job-id",
              payload.jobId || "browser-dev-job",
              "--mode",
              payload.mode || "auto"
            ]);
            let stderr = "";
            child.stderr.on("data", (chunk) => {
              stderr += String(chunk);
            });
            child.on("error", reject);
            child.on("close", (code) => {
              if (code === 0) resolvePromise();
              else reject(new Error(stderr.trim() || `parser_exit_${code}`));
            });
          });
          const parsed = JSON.parse(await readFile(outputPath, "utf8"));
          await rm(tempDir, { recursive: true, force: true });
          res.setHeader("content-type", "application/json; charset=utf-8");
          res.end(JSON.stringify(parsed));
        } catch (error) {
          res.statusCode = 500;
          res.setHeader("content-type", "application/json; charset=utf-8");
          res.end(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }));
        }
      });
    }
  };
}

export default defineConfig({
  plugins: [react(), devSourceParserPlugin()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true
  },
  envPrefix: ["VITE_", "TAURI_"]
});
