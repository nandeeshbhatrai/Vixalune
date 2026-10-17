import { useCallback, useEffect, useState } from "react";

export type ThemeChoice = "system" | "dark" | "light";
const KEY = "vixalune.theme";

function read(): ThemeChoice {
  try {
    const v = localStorage.getItem(KEY);
    return v === "dark" || v === "light" ? v : "system";
  } catch {
    return "system";
  }
}

function apply(choice: ThemeChoice) {
  const root = document.documentElement;
  if (choice === "system") delete root.dataset.theme;
  else root.dataset.theme = choice;
}

/** Apply the stored choice before first render to avoid a flash. */
export function initTheme() {
  apply(read());
}

export function useTheme(): [ThemeChoice, (c: ThemeChoice) => void] {
  const [choice, setChoice] = useState<ThemeChoice>(read);
  useEffect(() => apply(choice), [choice]);
  const set = useCallback((c: ThemeChoice) => {
    try {
      if (c === "system") localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, c);
    } catch {
      /* storage unavailable: the choice still applies for this session */
    }
    setChoice(c);
  }, []);
  return [choice, set];
}
