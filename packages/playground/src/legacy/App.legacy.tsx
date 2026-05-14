export function LegacyNotice() {
  return (
    <div className="min-h-screen flex flex-col items-center justify-center gap-4 bg-bg-base text-fg-primary p-8 text-center">
      <h1 className="text-2xl font-semibold">LabWired Playground (Legacy)</h1>
      <p className="text-fg-secondary max-w-[480px]">
        We&apos;ve upgraded the playground. The legacy shell is preserved here for two weeks as a fallback.
      </p>
      <a
        href="../"
        className="h-9 px-4 rounded-button bg-accent text-bg-base font-medium hover:bg-accent-hover"
      >
        Open the new playground →
      </a>
    </div>
  );
}
