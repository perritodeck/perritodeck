export declare const PROTOCOL: 1;

export type Settings = Record<string, unknown>;

export declare class Plugin {
  constructor(options?: { input?: NodeJS.ReadableStream; output?: NodeJS.WritableStream });
  /** The host's version, after it said hello. */
  hostVersion: string | null;
  /** The plugin's own settings, as saved on the Plugins page. */
  settings: Settings;
  /** Values with an argument that keys show, e.g. "up:192.168.1.10". */
  watching: Set<string>;
  onSettings(fn: (settings: Settings) => void | Promise<void>): this;
  onWatch(fn: (values: string[]) => void | Promise<void>): this;
  /** Run fn every `seconds` once started; errors are logged. */
  every(seconds: number, fn: () => void | Promise<void>, options?: { now?: boolean }): this;
  /** The arguments keys use with a value, e.g. hosts for "up". */
  watched(id: string): string[];
  /** Return a string to flash a warning on the key; throw to flash an error. */
  action(id: string, handler: (settings: Settings) => void | string | Promise<void | string>): this;
  onStart(fn: () => void | Promise<void>): this;
  /** Key titles show it as {plugin:<plugin id>:<value id>}. */
  setValue(id: string, value: string | number | boolean, arg?: string): void;
  log(message: string, level?: "info" | "warn" | "error"): void;
  /** Talk to the host until it says goodbye. */
  start(): Promise<void>;
}
