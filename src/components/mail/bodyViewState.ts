import type { InjectionKey, ShallowRef } from 'vue';

export interface BodyViewOption {
  value: string;
  label: string;
  title?: string;
}

/** The body's render modes as the message around it sees them. */
export interface BodyViewState {
  options: BodyViewOption[];
  mode: string;
  /** The mode the renderer opens this body in; the way back from a switch. */
  defaultMode: string;
  set(mode: string): void;
}

/**
 * Where MessageBodyViewSwitch reports the renderer's modes, so the message's
 * own chrome (its ⋯ sheet on the phone) can offer them. Null while the body
 * has no choice to offer.
 */
export const BODY_VIEW_STATE: InjectionKey<ShallowRef<BodyViewState | null>> = Symbol('bodyViewState');
