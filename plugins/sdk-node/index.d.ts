export declare const PROTOCOL: 1;

export type Settings = Record<string, unknown>;

export declare class Plugin {
  constructor(options?: { input?: NodeJS.ReadableStream; output?: NodeJS.WritableStream });
  /** The host's version, after it said hello. */
  hostVersion: string | null;
  /** Return a string to flash a warning on the key; throw to flash an error. */
  action(id: string, handler: (settings: Settings) => void | string | Promise<void | string>): this;
  onStart(fn: () => void | Promise<void>): this;
  /** Key titles show it as {plugin:<plugin id>:<value id>}. */
  setValue(id: string, value: string | number | boolean): void;
  log(message: string, level?: "info" | "warn" | "error"): void;
  /** Talk to the host until it says goodbye. */
  start(): Promise<void>;
}
