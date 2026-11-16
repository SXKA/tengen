/// <reference lib="webworker" />
/**
 * A search helper thread for the multi-threaded engine: a second instance of
 * the module on the same shared memory, with its own stack and thread-local
 * storage. It never returns from `tg_helper`.
 */

interface Setup {
  module: WebAssembly.Module;
  memory: WebAssembly.Memory;
  stackTop: number;
  tls: number;
  id: number;
}

const scope = self as unknown as DedicatedWorkerGlobalScope;

scope.onmessage = async (event: MessageEvent<Setup>) => {
  const { module, memory, stackTop, tls, id } = event.data;
  const instance = await WebAssembly.instantiate(module, {
    env: {
      memory,
      tg_now: () => performance.timeOrigin + performance.now(),
      tg_info: () => {},
    },
  });
  const exports = instance.exports as unknown as {
    __stack_pointer: WebAssembly.Global;
    __wasm_init_tls(ptr: number): void;
    tg_helper(id: number): void;
  };
  // The instance starts with the main thread's stack pointer; move it to
  // this thread's own stack before running any code.
  exports.__stack_pointer.value = stackTop;
  exports.__wasm_init_tls(tls);
  exports.tg_helper(id);
};
