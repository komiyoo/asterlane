export function FormMessage({ tone, children }: { tone: "error" | "info"; children: string }) {
  return (
    <p
      className={tone === "error" ? "form-error" : "hint"}
      role={tone === "error" ? "alert" : "status"}
    >
      {children}
    </p>
  );
}
