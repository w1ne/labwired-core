/** Inherited env with secrets stripped — compile/run subprocesses must
 *  never see BUILDER_SECRET.  Keeps PATH/HOME/asdf vars so the toolchain
 *  resolves on both asdf-shimmed dev boxes and plain production hosts. */
const SECRET_KEYS = ['BUILDER_SECRET'];

export function safeEnv(): NodeJS.ProcessEnv {
  const e: NodeJS.ProcessEnv = { ...process.env };
  for (const k of SECRET_KEYS) delete e[k];
  return e;
}
