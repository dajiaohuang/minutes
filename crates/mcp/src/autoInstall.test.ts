import { mkdtemp, readFile, stat, writeFile } from "fs/promises";
import { tmpdir } from "os";
import { basename, join } from "path";
import { createHash } from "crypto";
import { gzipSync } from "zlib";
import { describe, expect, it } from "vitest";

import {
  atomicReplaceMacSherpaFiles,
  downloadReleaseBinaryWithChecksum,
  extractMacSherpaArchive,
  extractZipWithPowerShell,
  findSha256ForAsset,
  installMacSherpaArchiveWithFallback,
  MACOS_BARE_BINARY,
  MACOS_SHERPA_ARCHIVE,
  MACOS_LEGACY_SHERPA_ARCHIVE,
  parseSha256Sums,
  ReleaseAssetUnavailableError,
} from "./autoInstall.js";

function sha256(input: string): string {
  return createHash("sha256").update(input).digest("hex");
}

function tarArchive(files: Array<{ name: string; contents: string }>): Buffer {
  const blocks: Buffer[] = [];
  for (const file of files) {
    const contents = Buffer.from(file.contents);
    const header = Buffer.alloc(512);
    header.write(file.name, 0, 100, "utf8");
    header.write(`${contents.length.toString(8).padStart(11, "0")}\0`, 124, 12, "ascii");
    header[156] = "0".charCodeAt(0);
    blocks.push(header, contents);
    const padding = (512 - (contents.length % 512)) % 512;
    if (padding) blocks.push(Buffer.alloc(padding));
  }
  blocks.push(Buffer.alloc(1024));
  return Buffer.concat(blocks);
}

describe("parseSha256Sums", () => {
  it("parses standard sha256sum output", () => {
    const mac = "a".repeat(64);
    const linux = "b".repeat(64);

    expect(
      parseSha256Sums(`
${mac}  minutes-macos-arm64
${linux} *minutes-linux-x64
`)
    ).toEqual([
      { filename: "minutes-macos-arm64", sha256: mac },
      { filename: "minutes-linux-x64", sha256: linux },
    ]);
  });

  it("ignores blank, comment, and malformed lines", () => {
    const windows = "C".repeat(64);
    expect(
      parseSha256Sums(`
# release checksums
not a checksum

${windows}  minutes-windows-x64.exe
`)
    ).toEqual([{ filename: "minutes-windows-x64.exe", sha256: windows.toLowerCase() }]);
  });

  it("finds entries by basename for nested artifact paths", () => {
    const checksum = "d".repeat(64);
    expect(
      findSha256ForAsset(
        `${checksum}  dist/minutes-linux-x64\n`,
        "minutes-linux-x64"
      )
    ).toBe(checksum);
  });
});

describe("downloadReleaseBinaryWithChecksum", () => {
  it("downloads the sums first, verifies the binary, and installs it", async () => {
    const dir = await mkdtemp(join(tmpdir(), "minutes-mcp-install-"));
    const targetPath = join(dir, "minutes");
    const payload = "verified cli";
    const checksum = sha256(payload);
    const calls: string[] = [];

    const execFileAsync = async (_file: string, args: readonly string[]) => {
      const outputPath = args[2] as string;
      const url = args[3] as string;
      calls.push(url);
      if (url.endsWith("/SHA256SUMS.txt")) {
        await writeFile(outputPath, `${checksum}  minutes-linux-x64\n`);
      } else if (url.endsWith("/minutes-linux-x64")) {
        await writeFile(outputPath, payload);
      }
    };

    await downloadReleaseBinaryWithChecksum({
      binaryName: "minutes-linux-x64",
      targetPath,
      execFileAsync,
      baseUrl: "https://example.test/download",
    });

    expect(calls).toEqual([
      "https://example.test/download/SHA256SUMS.txt",
      "https://example.test/download/minutes-linux-x64",
    ]);
    await expect(readFile(targetPath, "utf8")).resolves.toBe(payload);
  });

  it("aborts and leaves no target binary when checksum verification fails", async () => {
    const dir = await mkdtemp(join(tmpdir(), "minutes-mcp-install-"));
    const targetPath = join(dir, "minutes");

    const execFileAsync = async (_file: string, args: readonly string[]) => {
      const outputPath = args[2] as string;
      const url = args[3] as string;
      if (url.endsWith("/SHA256SUMS.txt")) {
        await writeFile(outputPath, `${"0".repeat(64)}  minutes-linux-x64\n`);
      } else if (url.endsWith("/minutes-linux-x64")) {
        await writeFile(outputPath, "bad payload");
      }
    };

    await expect(
      downloadReleaseBinaryWithChecksum({
        binaryName: "minutes-linux-x64",
        targetPath,
        execFileAsync,
        baseUrl: "https://example.test/download",
      })
    ).rejects.toThrow("checksum mismatch");
    await expect(stat(targetPath)).rejects.toMatchObject({ code: "ENOENT" });
  });
});

describe("macOS sherpa archive installation", () => {
  it("extracts the fixed binary and plugin layout from a fixture tar.gz", async () => {
    const dir = await mkdtemp(join(tmpdir(), "minutes-mcp-sherpa-tar-"));
    const archivePath = join(dir, MACOS_SHERPA_ARCHIVE);
    const archive = tarArchive([
      { name: "minutes-macos-arm64-sherpa/minutes", contents: "fixture cli" },
      {
        name: "minutes-macos-arm64-sherpa/libminutes_sherpa.dylib",
        contents: "fixture plugin",
      },
      {
        name: "minutes-macos-arm64-sherpa/libsherpa-onnx-c-api.dylib",
        contents: "fixture sherpa library",
      },
      {
        name: "minutes-macos-arm64-sherpa/libonnxruntime.dylib",
        contents: "fixture ort library",
      },
    ]);
    await writeFile(archivePath, gzipSync(archive));

    const extracted = await extractMacSherpaArchive({
      archivePath,
      destDir: join(dir, "out"),
    });

    await expect(readFile(extracted.binaryPath, "utf8")).resolves.toBe("fixture cli");
    await expect(readFile(extracted.pluginPath, "utf8")).resolves.toBe("fixture plugin");
    expect(extracted.libraryPaths.map((path) => basename(path))).toEqual([
      "libsherpa-onnx-c-api.dylib",
      "libonnxruntime.dylib",
    ]);
    await expect(readFile(extracted.libraryPaths[1], "utf8")).resolves.toBe("fixture ort library");
  });

  it("installs the legacy archive when the shared asset is absent from an older release", async () => {
    const dir = await mkdtemp(join(tmpdir(), "minutes-mcp-legacy-"));
    const downloads: string[] = [];
    const result = await installMacSherpaArchiveWithFallback({
      installDir: dir,
      execFileAsync: async () => undefined,
      downloadAsset: async ({ binaryName, targetPath }) => {
        downloads.push(binaryName);
        if (binaryName === MACOS_SHERPA_ARCHIVE) {
          throw new ReleaseAssetUnavailableError(binaryName, "older release");
        }
        await writeFile(targetPath, gzipSync(tarArchive([
          { name: "minutes-macos-arm64-sherpa/minutes", contents: "legacy cli" },
          { name: "minutes-macos-arm64-sherpa/libminutes_sherpa.dylib", contents: "static plugin" },
        ])));
      },
    });
    expect(downloads).toEqual([MACOS_SHERPA_ARCHIVE, MACOS_LEGACY_SHERPA_ARCHIVE]);
    expect(result).toEqual({ assetName: MACOS_LEGACY_SHERPA_ARCHIVE, usedFallback: true });
    await expect(readFile(join(dir, "libminutes_sherpa.dylib"), "utf8")).resolves.toBe("static plugin");
    await expect(stat(join(dir, "libonnxruntime.dylib"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("does not fall back after checksum or malformed-archive errors", async () => {
    for (const corruptedDownload of [false, true]) {
      const dir = await mkdtemp(join(tmpdir(), "minutes-mcp-bad-shared-"));
      const downloads: string[] = [];
      await expect(installMacSherpaArchiveWithFallback({
        installDir: dir,
        execFileAsync: async () => undefined,
        downloadAsset: async ({ binaryName, targetPath }) => {
          downloads.push(binaryName);
          if (corruptedDownload) throw new Error("checksum mismatch");
          await writeFile(targetPath, gzipSync(tarArchive([
            { name: "minutes-macos-arm64-sherpa/minutes", contents: "cli" },
          ])));
        },
      })).rejects.toThrow(corruptedDownload ? "checksum mismatch" : "missing");
      expect(downloads).toEqual([MACOS_SHERPA_ARCHIVE]);
    }
  });

  it("rejects an archive that is missing a shared library", async () => {
    const dir = await mkdtemp(join(tmpdir(), "minutes-mcp-sherpa-tar-missing-"));
    const archivePath = join(dir, MACOS_SHERPA_ARCHIVE);
    await writeFile(
      archivePath,
      gzipSync(
        tarArchive([
          { name: "minutes-macos-arm64-sherpa/minutes", contents: "fixture cli" },
          { name: "minutes-macos-arm64-sherpa/libminutes_sherpa.dylib", contents: "plugin" },
          { name: "minutes-macos-arm64-sherpa/libsherpa-onnx-c-api.dylib", contents: "sherpa" },
        ])
      )
    );
    await expect(
      extractMacSherpaArchive({ archivePath, destDir: join(dir, "out") })
    ).rejects.toThrow("missing minutes-macos-arm64-sherpa/libonnxruntime.dylib");
  });

  it("falls back to the bare asset only when the archive is unavailable", async () => {
    const installDir = await mkdtemp(join(tmpdir(), "minutes-mcp-sherpa-fallback-"));
    const downloads: string[] = [];
    const downloadAsset = async (input: {
      binaryName: string;
      targetPath: string;
    }): Promise<void> => {
      downloads.push(input.binaryName);
      if (input.binaryName !== MACOS_BARE_BINARY) {
        throw new ReleaseAssetUnavailableError(input.binaryName, "fixture archive is absent");
      }
      await writeFile(input.targetPath, "bare cli");
    };

    const result = await installMacSherpaArchiveWithFallback({
      installDir,
      execFileAsync: async () => undefined,
      downloadAsset,
    });

    expect(downloads).toEqual([MACOS_SHERPA_ARCHIVE, MACOS_LEGACY_SHERPA_ARCHIVE, MACOS_BARE_BINARY]);
    expect(result).toEqual({ assetName: MACOS_BARE_BINARY, usedFallback: true });
    await expect(readFile(join(installDir, "minutes"), "utf8")).resolves.toBe("bare cli");
  });

  it("keeps the previous binary and plugin when staged --version fails", async () => {
    const installDir = await mkdtemp(join(tmpdir(), "minutes-mcp-sherpa-atomic-"));
    const targetBinary = join(installDir, "minutes");
    const targetPlugin = join(installDir, "libminutes_sherpa.dylib");
    const targetLibrary = join(installDir, "libonnxruntime.dylib");
    await writeFile(targetBinary, "old cli");
    await writeFile(targetPlugin, "old plugin");
    await writeFile(targetLibrary, "old ort");

    await expect(
      installMacSherpaArchiveWithFallback({
        installDir,
        execFileAsync: async () => undefined,
        downloadAsset: async (input: { targetPath: string }) => {
          await writeFile(input.targetPath, "verified archive fixture");
        },
        extractArchive: async ({ destDir }) => {
          const binaryPath = join(destDir, "minutes");
          const pluginPath = join(destDir, "libminutes_sherpa.dylib");
          const libraryPath = join(destDir, "libonnxruntime.dylib");
          await writeFile(binaryPath, "new cli");
          await writeFile(pluginPath, "new plugin");
          await writeFile(libraryPath, "new ort");
          return { binaryPath, pluginPath, libraryPaths: [libraryPath] };
        },
        verifyExecutable: async () => {
          throw new Error("--version failed");
        },
      })
    ).rejects.toThrow("--version failed");

    await expect(readFile(targetBinary, "utf8")).resolves.toBe("old cli");
    await expect(readFile(targetPlugin, "utf8")).resolves.toBe("old plugin");
    await expect(readFile(targetLibrary, "utf8")).resolves.toBe("old ort");
  });

  it("installs the plugin with its libraries and restores them all if the binary swap fails", async () => {
    const installDir = await mkdtemp(join(tmpdir(), "minutes-mcp-sherpa-libs-"));
    const staged = await mkdtemp(join(tmpdir(), "minutes-mcp-sherpa-staged-"));
    const files = {
      plugin: join(installDir, "libminutes_sherpa.dylib"),
      library: join(installDir, "libonnxruntime.dylib"),
    };
    await writeFile(files.plugin, "old plugin");
    await writeFile(files.library, "old ort");
    const stagedPlugin = join(staged, "libminutes_sherpa.dylib");
    const stagedLibrary = join(staged, "libonnxruntime.dylib");
    await writeFile(stagedPlugin, "new plugin");
    await writeFile(stagedLibrary, "new ort");

    await expect(
      atomicReplaceMacSherpaFiles({
        stagedBinaryPath: join(staged, "does-not-exist"),
        stagedPluginPath: stagedPlugin,
        stagedLibraryPaths: [stagedLibrary],
        targetBinaryPath: join(installDir, "minutes"),
        targetPluginPath: files.plugin,
        backupDir: staged,
      })
    ).rejects.toThrow();

    await expect(readFile(files.plugin, "utf8")).resolves.toBe("old plugin");
    await expect(readFile(files.library, "utf8")).resolves.toBe("old ort");

    await writeFile(join(staged, "minutes"), "new cli");
    await writeFile(stagedPlugin, "new plugin");
    await writeFile(stagedLibrary, "new ort");
    await atomicReplaceMacSherpaFiles({
      stagedBinaryPath: join(staged, "minutes"),
      stagedPluginPath: stagedPlugin,
      stagedLibraryPaths: [stagedLibrary],
      targetBinaryPath: join(installDir, "minutes"),
      targetPluginPath: files.plugin,
      backupDir: staged,
    });
    await expect(readFile(files.plugin, "utf8")).resolves.toBe("new plugin");
    await expect(readFile(files.library, "utf8")).resolves.toBe("new ort");
    await expect(readFile(join(installDir, "minutes"), "utf8")).resolves.toBe("new cli");
  });
});

describe("extractZipWithPowerShell", () => {
  function capture() {
    const calls: Array<{
      file: string;
      args: readonly string[];
      options?: { timeout?: number; env?: NodeJS.ProcessEnv };
    }> = [];
    const execFileAsync = async (
      file: string,
      args: readonly string[],
      options?: { timeout?: number; env?: NodeJS.ProcessEnv }
    ) => {
      calls.push({ file, args, options });
      return undefined;
    };
    return { calls, execFileAsync };
  }

  it("passes both paths through the environment, not the command text", async () => {
    const { calls, execFileAsync } = capture();
    await extractZipWithPowerShell({
      archivePath: "C:\\Users\\qa\\.minutes\\bin\\minutes-windows-x64.zip",
      destDir: "C:\\Users\\qa\\.minutes\\bin",
      execFileAsync,
    });

    expect(calls).toHaveLength(1);
    const [call] = calls;
    expect(call.file).toBe("powershell");
    expect(call.options?.env?.MINUTES_ZIP_PATH).toBe(
      "C:\\Users\\qa\\.minutes\\bin\\minutes-windows-x64.zip"
    );
    expect(call.options?.env?.MINUTES_ZIP_DEST).toBe("C:\\Users\\qa\\.minutes\\bin");
  });

  it("never reads $args, which -Command leaves empty", async () => {
    const { calls, execFileAsync } = capture();
    await extractZipWithPowerShell({
      archivePath: "C:\\tmp\\a.zip",
      destDir: "C:\\tmp\\out",
      execFileAsync,
    });

    const { args } = calls[0];
    const commandIndex = args.indexOf("-Command");
    expect(commandIndex).toBeGreaterThanOrEqual(0);
    const script = args[commandIndex + 1];

    // `powershell -Command` appends trailing arguments to the command text and
    // leaves $args empty; only -File binds them. Reading $args[0] made every
    // Windows install fail with "argument is null or empty".
    expect(script).not.toContain("$args");
    expect(script).toContain("$env:MINUTES_ZIP_PATH");
    expect(script).toContain("$env:MINUTES_ZIP_DEST");

    // Nothing may follow the script, or it lands in the command text.
    expect(args).toHaveLength(commandIndex + 2);
  });

  it("keeps an apostrophe home directory out of the command text", async () => {
    const { calls, execFileAsync } = capture();
    const home = "C:\\Users\\O'Brien\\.minutes\\bin";
    await extractZipWithPowerShell({
      archivePath: `${home}\\minutes-windows-x64.zip`,
      destDir: home,
      execFileAsync,
    });

    const { args, options } = calls[0];
    const script = args[args.indexOf("-Command") + 1];
    expect(script).not.toContain("O'Brien");
    expect(options?.env?.MINUTES_ZIP_DEST).toBe(home);
  });
});
