import { readFile, realpath } from 'node:fs/promises';
import path from 'node:path';
import readline from 'node:readline';
import { fileURLToPath } from 'node:url';
import { normalizedPath, verifyProfile } from './profile.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const emit = event => process.stdout.write(`${JSON.stringify(event)}\n`);

class ObserverHost {
  constructor() {
    this.stopped = false;
    this.started = false;
    this.session = null;
    this.script = null;
    this.device = null;
    this.spawnedPid = null;
    this.resumed = false;
  }

  async start(config) {
    if (this.started) throw new Error('Observer has already started');
    this.started = true;
    if (process.platform !== 'win32') throw new Error('Automatic Origins observation currently supports Windows');
    const profile = await verifyProfile(config);
    if (this.stopped) return;
    const frida = await import('frida');
    this.device = await frida.getLocalDevice();
    let pid = config.pid;
    if (config.launch === true) {
      if (pid !== undefined) throw new Error('Choose an existing PID or launch, not both');
      const argv = config.argv ?? [];
      if (!Array.isArray(argv) || argv.some(value => typeof value !== 'string')) throw new Error('Client arguments must be strings');
      // Frida creates the owned process suspended. Module observation is
      // installed before resume, including Xtras loaded later at game startup.
      pid = await this.device.spawn([profile.executable, ...argv], { cwd: path.dirname(profile.executable), stdio: frida.Stdio.Pipe });
      this.spawnedPid = pid;
    } else if (pid === undefined) {
      emit({ type: 'status', state: 'waiting_for_client' });
      while (!this.stopped && pid === undefined) {
        const processes = await this.device.enumerateProcesses({ scope: frida.Scope.Full });
        for (const candidate of processes) {
          const candidatePath = candidate.parameters.path;
          if (typeof candidatePath !== 'string') continue;
          try {
            if (normalizedPath(await realpath(candidatePath)) === normalizedPath(profile.executable)) {
              pid = candidate.pid;
              break;
            }
          } catch (_) { /* Exited or inaccessible unrelated process. */ }
        }
        if (pid === undefined) await new Promise(resolve => setTimeout(resolve, 500));
      }
      if (this.stopped) return;
    }
    if (!Number.isInteger(pid) || pid <= 0) throw new Error('Invalid client PID');
    const processes = await this.device.enumerateProcesses({ pids: [pid], scope: frida.Scope.Full });
    const observedPath = processes[0]?.parameters.path;
    if (typeof observedPath !== 'string' || normalizedPath(await realpath(observedPath)) !== normalizedPath(profile.executable)) throw new Error('Selected process is not the validated client executable');
    if (this.stopped) return;
    this.session = await this.device.attach(pid);
    this.session.detached.connect(reason => emit({ type: 'status', state: 'detached', pid, reason }));
    const source = await readFile(path.join(here, 'origins-agent.cjs'), 'utf8');
    this.script = await this.session.createScript(source, { name: 'hablog-origins-observer', runtime: frida.ScriptRuntime.QJS });
    this.script.logHandler = (_level, _text) => { /* Never forward arbitrary target console output. */ };
    this.script.message.connect(message => {
      if (message.type === 'send') {
        const payload = message.payload;
        if (payload?.type === 'batch' && Array.isArray(payload.events)) payload.events.forEach(emit);
        else emit(payload);
      } else if (message.type === 'error') {
        emit({ type: 'error', code: 'agent_error', message: message.description });
      }
    });
    await this.script.load();
    const status = await this.script.exports.configure(profile);
    emit({ type: 'status', ...status, capture_mode: profile.captureMode });
    if (this.stopped) return;
    if (this.spawnedPid !== null) {
      await this.device.resume(this.spawnedPid);
      this.resumed = true;
      emit({ type: 'status', state: 'client_resumed', pid });
    }
  }

  async setCaptureMode(mode) {
    if (this.script === null) throw new Error('Observer has not attached');
    const result = await this.script.exports.setcapturemode(mode);
    emit({ type: 'status', ...result });
  }

  async stop() {
    this.stopped = true;
    if (this.script !== null && !this.script.isDestroyed) {
      try { await this.script.exports.dispose(); } catch (_) { /* Target may have exited. */ }
      try { await this.script.unload(); } catch (_) { /* Target may have exited. */ }
    }
    if (this.session !== null && !this.session.isDetached()) {
      try { await this.session.detach(); } catch (_) { /* Target may have exited. */ }
    }
    // Never leave a client this companion launched suspended after a failure.
    // Detaching leaves the ordinary client running, without killing it.
    if (this.spawnedPid !== null && !this.resumed && this.device !== null) {
      try { await this.device.resume(this.spawnedPid); this.resumed = true; } catch (_) { /* Client may have exited. */ }
    }
  }
}

async function main() {
  const observer = new ObserverHost();
  const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity, terminal: false });
  let starting = null;
  const fail = error => emit({ type: 'error', code: 'observer_failed', message: String(error.message ?? error) });
  process.once('SIGINT', () => input.close());
  process.once('SIGTERM', () => input.close());
  emit({ type: 'status', state: 'companion_ready', protocol: 1 });
  for await (const line of input) {
    try {
      if (line.length > 1024 * 1024) throw new Error('Control message is too large');
      const message = JSON.parse(line);
      if (message.command === 'start') {
        if (starting !== null) throw new Error('Observer has already started');
        starting = observer.start(message.config).catch(async error => { fail(error); await observer.stop(); });
      } else if (message.command === 'capture_mode') {
        await observer.setCaptureMode(message.mode);
      } else if (message.command === 'stop') {
        await observer.stop();
        input.close();
        break;
      } else {
        throw new Error('Unknown companion command');
      }
    } catch (error) { fail(error); }
  }
  await observer.stop();
  if (starting !== null) await starting;
  await observer.stop();
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { emit({ type: 'error', code: 'host_error', message: String(error.message ?? error) }); process.exitCode = 1; });
}
