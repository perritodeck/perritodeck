export declare const PROTOCOL: 1;

export declare const DATA_ENV: "PERRITODECK_PLUGIN_DATA";
/** Where plugin files go when the host didn't set PERRITODECK_PLUGIN_DATA. */
export declare function defaultDataDir(
  name: string,
  options?: { platform?: string; env?: Record<string, string | undefined>; home?: string },
): string;

export type Settings = Record<string, unknown>;

export declare class Plugin {
  constructor(options?: { input?: NodeJS.ReadableStream; output?: NodeJS.WritableStream });
  /** The host's version, after it said hello. */
  hostVersion: string | null;
  /** The plugin's own settings, as saved on the Plugins page. */
  settings: Settings;
  /** Values with an argument that keys show, e.g. "up:192.168.1.10". */
  watching: Set<string>;
  /** A folder for the plugin's own files (PERRITODECK_PLUGIN_DATA, or a per-user
   * fallback), created the first time it is read. */
  readonly dataDir: string;
  onSettings(fn: (settings: Settings) => void | Promise<void>): this;
  onWatch(fn: (values: string[]) => void | Promise<void>): this;
  /** Run fn every `seconds` once started; errors are logged. Ticks never overlap:
   * the next one is scheduled `seconds` after the previous one settled. */
  every(seconds: number, fn: () => void | Promise<void>, options?: { now?: boolean }): this;
  /** The arguments keys use with a value, e.g. hosts for "up". */
  watched(id: string): string[];
  /** Return a string to flash a warning on the key; throw to flash an error. */
  action(id: string, handler: (settings: Settings) => void | string | Promise<void | string>): this;
  onStart(fn: () => void | Promise<void>): this;
  /** Key titles show it as {plugin:<plugin id>:<value id>}. */
  setValue(id: string, value: string | number | boolean, arg?: string): void;
  log(message: string, level?: "info" | "warn" | "error"): void;
  /** Open a web address, file, folder or app link for the person, as double-clicking
   * would. The host opens it, so it keeps running after the plugin stops (on Windows,
   * what a plugin starts itself ends with it). Rejects with the reason if it couldn't. */
  openForUser(target: string): Promise<void>;
  /** Start a program for the person, e.g. startForUser("code", ["."]); the host starts
   * it, as for openForUser. */
  startForUser(program: string, args?: string[]): Promise<void>;
  /** Talk to the host until it says goodbye. */
  start(): Promise<void>;
}
