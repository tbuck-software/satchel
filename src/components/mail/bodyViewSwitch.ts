import type { Component, InjectionKey } from 'vue';

/**
 * What stands where EmailRenderer would draw its Original / Auto-dark / Text
 * switch, handed in by the message around it (MessageBubble provides
 * MessageBodyViewSwitch.vue). The phone moves the choice out of the body;
 * the message-file viewer and the topic feed provide nothing and keep the
 * segmented control, so none of that code enters their bundles.
 *
 * The component receives the switch's props (`options`, `modelValue`,
 * `ariaLabel`), plus `defaultValue`, the mode the renderer opens this body
 * in, and emits `update:modelValue`.
 */
export const BODY_VIEW_SWITCH: InjectionKey<Component> = Symbol('bodyViewSwitch');
