// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';
import type { BackendOutgoingAttachment } from '@/types/composer';
import AttachmentDropZone from './AttachmentDropZone.vue';

/** jsdom's File has no `arrayBuffer`; `ready` holds the read back until released. */
function file(name: string, ready: Promise<void> = Promise.resolve()): File {
  const created = new File([name], name, { type: 'text/plain' });
  Object.defineProperty(created, 'arrayBuffer', {
    value: async () => {
      await ready;
      return new TextEncoder().encode(name).buffer;
    },
  });
  return created;
}

function unreadable(name: string): File {
  const broken = new File([name], name, { type: 'text/plain' });
  Object.defineProperty(broken, 'arrayBuffer', {
    value: () => Promise.reject(new DOMException('gone', 'NotReadableError')),
  });
  return broken;
}

/** A parent that applies `update:modelValue` the way `v-model` does. */
function mountWithModel(initial: BackendOutgoingAttachment[] = []) {
  const wrapper = mount(AttachmentDropZone, {
    props: {
      modelValue: initial,
      'onUpdate:modelValue': (value: BackendOutgoingAttachment[]) => {
        void wrapper.setProps({ modelValue: value });
      },
    },
  });
  return wrapper;
}

function drop(wrapper: ReturnType<typeof mountWithModel>, files: File[]) {
  return wrapper.trigger('drop', { dataTransfer: { files } });
}

const names = (wrapper: ReturnType<typeof mountWithModel>) =>
  (wrapper.props('modelValue') as BackendOutgoingAttachment[]).map((entry) => entry.filename);

describe('AttachmentDropZone', () => {
  it('keeps the readable files of a batch when one cannot be read', async () => {
    const wrapper = mountWithModel();

    await drop(wrapper, [file('a.txt'), unreadable('broken.txt'), file('b.txt')]);
    await flushPromises();

    expect(names(wrapper)).toEqual(['a.txt', 'b.txt']);
    expect(wrapper.text()).toContain('“broken.txt” could not be read.');
  });

  it('does not let a second drop overwrite a slower first one', async () => {
    const wrapper = mountWithModel();
    let release!: () => void;
    const slow = new Promise<void>((resolve) => { release = resolve; });

    await drop(wrapper, [file('first.txt', slow)]);
    await drop(wrapper, [file('second.txt')]);
    await flushPromises();
    release();
    await flushPromises();

    expect(names(wrapper)).toEqual(['first.txt', 'second.txt']);
  });
});
