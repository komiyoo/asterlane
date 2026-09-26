export interface DownloadEnv {
  createObjectURL: (blob: Blob) => string;
  revokeObjectURL: (url: string) => void;
  click: (url: string, filename: string) => void;
}

export function downloadYaml(text: string, env: DownloadEnv = browserDownloadEnv()): void {
  const url = env.createObjectURL(new Blob([text], { type: "text/yaml" }));
  try {
    env.click(url, "gateway-export.yaml");
  } finally {
    env.revokeObjectURL(url);
  }
}

function browserDownloadEnv(): DownloadEnv {
  return {
    createObjectURL: (blob) => URL.createObjectURL(blob),
    revokeObjectURL: (url) => URL.revokeObjectURL(url),
    click: (url, filename) => {
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = filename;
      anchor.click();
    },
  };
}
