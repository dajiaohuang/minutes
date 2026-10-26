import type { Metadata } from "next";
import { APPLE_SILICON_DOWNLOAD_PATH } from "@/lib/downloads";

export const metadata: Metadata = {
  title: "Minutes won’t open? Reinstall and keep your meetings",
  description: "Download and reinstall Minutes without opening the app. Keep your meeting files, settings and speech models.",
  alternates: { canonical: "https://useminutes.app/download/recover" },
};

export default function RecoveryPage() {
  return (
    <main className="mx-auto max-w-[640px] px-6 py-16 text-[var(--text)]">
      <h1 className="font-serif text-[40px] leading-tight">Minutes won’t open?</h1>
      <p className="mt-6 text-[17px] leading-7 text-[var(--text-secondary)]">
        You can reinstall Minutes without opening it. Your meeting files, settings and downloaded speech models are stored separately from the application.
      </p>
      <a className="mt-6 inline-block rounded-[6px] bg-[var(--text)] px-5 py-3 font-medium text-[var(--bg)] focus-visible:outline-2 focus-visible:outline-offset-4" href={APPLE_SILICON_DOWNLOAD_PATH}>
        Download Minutes for Mac (Apple Silicon)
      </a>
      <ol className="mt-8 list-decimal space-y-4 pl-6 text-[17px] leading-7">
        <li>Quit Minutes if it is running.</li>
        <li>Open the downloaded DMG.</li>
        <li>Drag Minutes into Applications. Choose <strong>Replace</strong> when asked.</li>
        <li>Open Minutes from Applications.</li>
      </ol>
      <p className="mt-6 text-[17px] leading-7 text-[var(--text-secondary)]">
        Replacing the application keeps your existing meeting files, settings and models. You do not need to remove them or start setup again.
      </p>
      <p className="mt-8 border-t border-[var(--border)] pt-6 text-[17px] leading-7">
        <strong>“Minutes quit unexpectedly” after an update?</strong> Try reopening Minutes first. The update may already have installed before the old process exited.
      </p>
      <p className="mt-6 text-[17px] leading-7 text-[var(--text-secondary)]">
        Still unable to open it? <a className="underline underline-offset-4" href="https://github.com/silverstein/minutes/discussions">Get help from the Minutes community</a>. Include your Mac model and macOS version so we can narrow down the cause.
      </p>
    </main>
  );
}
