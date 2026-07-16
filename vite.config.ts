import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

type PythonCommand = {
  command: string;
  args: string[];
};

function splitCommandLine(value: string): string[] {
  const parts: string[] = [];
  let current = "";
  let quote: string | null = null;
  for (const char of value.trim()) {
    if ((char === '"' || char === "'") && !quote) {
      quote = char;
      continue;
    }
    if (char === quote) {
      quote = null;
      continue;
    }
    if (/\s/.test(char) && !quote) {
      if (current) {
        parts.push(current);
        current = "";
      }
      continue;
    }
    current += char;
  }
  if (current) parts.push(current);
  return parts;
}

function pythonCommandFromEnv(): PythonCommand | null {
  const fromEnv = process.env.EPIC8_PYTHON || process.env.EPIC8_UNIFIED_PYTHON;
  if (!fromEnv) return null;
  if (existsSync(fromEnv)) {
    return { command: resolve(fromEnv), args: [] };
  }
  const [command, ...args] = splitCommandLine(fromEnv);
  return command ? { command, args } : null;
}

function canRun(command: string, args: string[] = []): boolean {
  const result = spawnSync(command, [...args, "--version"], {
    encoding: "utf8",
    timeout: 10000,
    stdio: ["ignore", "pipe", "pipe"]
  });
  return result.status === 0 || result.status === 1;
}

function resolvePythonCommand(): PythonCommand {
  const candidates = [
    pythonCommandFromEnv(),
    ...(process.platform === "win32"
      ? [
          { command: "py", args: ["-3"] },
          { command: "python", args: [] }
        ]
      : [
          { command: "python3", args: [] },
          { command: "python", args: [] }
        ])
  ].filter((candidate): candidate is PythonCommand => Boolean(candidate));

  for (const candidate of candidates) {
    if (canRun(candidate.command, candidate.args)) return candidate;
  }
  return candidates[0] ?? { command: "python3", args: [] };
}

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
          const python = resolvePythonCommand();
          await new Promise<void>((resolvePromise, reject) => {
            const child = spawn(python.command, [
              ...python.args,
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
