(function () {
  'use strict';

  const appPath = '/usr/palm/applications/org.arc-restore.app/scripts/control.sh';
  const broker = 'luna://org.webosbrew.hbchannel.service/exec';
  const buttons = ['enable', 'disable', 'status'].map(id => document.getElementById(id));
  const badge = document.getElementById('badge');
  const message = document.getElementById('message');
  const route = document.getElementById('route');
  const previous = document.getElementById('previous');
  const dot = document.getElementById('dot');
  const watcher = document.getElementById('watcher');
  const events = document.getElementById('events');
  const labels = {
    running: 'Enabled',
    disabled: 'Disabled',
    stopped: 'Stopped',
    uninstalled: 'Not installed'
  };
  const outputs = {
    external_arc: 'HDMI ARC / eARC',
    external_optical: 'Optical',
    external_speaker: 'External speaker',
    tv_speaker: 'TV speaker',
    bt_soundbar: 'Bluetooth speaker',
    tv_speaker_bluetooth: 'TV + Bluetooth',
    headphone: 'Headphones',
    lineout: 'Line out'
  };
  const details = {
    Bluetooth: 'Bluetooth is active. Waiting for it to disconnect.',
    CheckingRestore: 'Restore requested. Checking Sound Out…',
    Retrying: 'Restore request failed. Retrying…',
    RestoreFailed: 'Sound Out did not return after three attempts.',
    Restored: 'Previous output restored.',
    Waiting: 'Watching for Bluetooth audio.'
  };
  const detailTones = {
    Bluetooth: 'info',
    CheckingRestore: 'warn',
    Retrying: 'warn',
    RestoreFailed: 'danger',
    Restored: 'ok',
    Waiting: 'ok'
  };
  const feeds = {
    'settings+volume': 'Instantly',
    settings: 'Instantly (settings only)',
    volume: 'Instantly (audio only)',
    polling: 'Every 2 seconds'
  };
  let busy = false;

  // The notice takes the tone of the latest message; the status dot stays
  // green while the watcher runs unless something needs attention.
  function setTone(tone, running) {
    message.dataset.tone = tone;
    if (tone === 'warn' || tone === 'danger') dot.dataset.tone = tone;
    else if (running) dot.dataset.tone = 'ok';
    else delete dot.dataset.tone;
  }

  function invoke(action) {
    const command =
      'for mount in /media/developer/apps /media/cryptofs/apps; do ' +
      'script="$mount' + appPath + '"; ' +
      'if [ -r "$script" ]; then exec /bin/sh "$script" ' + action + '; fi; ' +
      'done; echo "ARC Restore app files could not be located." >&2; exit 1';

    return new Promise((resolve, reject) => {
      if (typeof window.PalmServiceBridge !== 'function') {
        reject(new Error('Open ARC Restore on your rooted LG TV.'));
        return;
      }
      const bridge = new window.PalmServiceBridge();
      const timer = window.setTimeout(() => {
        bridge.cancel();
        reject(new Error('Homebrew Channel did not respond. Open Homebrew Channel and try again.'));
      }, 12000);

      bridge.onservicecallback = raw => {
        window.clearTimeout(timer);
        try {
          const reply = JSON.parse(raw);
          if (reply.returnValue === false || reply.error || typeof reply.stdoutString !== 'string') {
            throw new Error(reply.stderrString || reply.errorText ||
              (typeof reply.error === 'string' && reply.error) ||
              'Homebrew Channel root service is unavailable.');
          }
          const fields = Object.create(null);
          reply.stdoutString.trim().split('\n').forEach(line => {
            const equals = line.indexOf('=');
            if (equals > 0) fields[line.slice(0, equals)] = line.slice(equals + 1);
          });
          if (!Object.prototype.hasOwnProperty.call(labels, fields.state)) {
            throw new Error(reply.stderrString || 'No status received from the watcher.');
          }
          resolve(fields);
        } catch (error) {
          reject(error);
        }
      };

      try {
        bridge.call(broker, JSON.stringify({command}));
      } catch (error) {
        window.clearTimeout(timer);
        reject(error);
      }
    });
  }

  function display(fields) {
    badge.textContent = labels[fields.state];
    badge.dataset.state = fields.state;
    const currentOutput = fields.route || fields.configured;
    route.textContent = outputs[currentOutput] || currentOutput || '—';
    previous.textContent = outputs[fields.previous] || fields.previous || '—';

    const running = fields.state === 'running';
    watcher.textContent = !running ? '—' : fields.watcher === 'native' ? 'Native (Rust)' : 'Shell script';
    events.textContent = !running ? '—' : feeds[fields.events] || 'Every 2 seconds';

    let statusText;
    let tone;
    switch (fields.state) {
      case 'running':
        statusText = details[fields.detail] || 'Watcher is starting…';
        tone = detailTones[fields.detail] || 'neutral';
        break;
      case 'stopped':
        statusText = 'The boot hook is enabled. Select Install & Enable to restart the watcher.';
        tone = 'warn';
        break;
      case 'disabled':
        statusText = 'The watcher is off and will stay off after reboot.';
        tone = 'neutral';
        break;
      default:
        statusText = 'Select Install & Enable while your usual Sound Out is active.';
        tone = 'neutral';
    }
    if (fields.legacy === 'active') {
      statusText += ' An older boot hook is still enabled.';
      if (tone !== 'danger') tone = 'warn';
    }
    message.textContent = statusText;
    setTone(tone, running);
  }

  async function act(action, quiet) {
    if (busy) return;
    busy = true;
    if (!quiet) {
      buttons.forEach(button => button.setAttribute('aria-disabled', 'true'));
      if (action !== 'status') {
        message.textContent = action === 'enable' ? 'Starting watcher…' : 'Stopping watcher…';
        setTone('neutral', badge.dataset.state === 'running');
      }
    }
    try {
      display(await invoke(action));
    } catch (error) {
      badge.dataset.state = 'error';
      badge.textContent = 'Unavailable';
      route.textContent = '—';
      previous.textContent = '—';
      watcher.textContent = '—';
      events.textContent = '—';
      message.textContent = error.message.trim();
      setTone('danger');
    } finally {
      busy = false;
      buttons.forEach(button => button.removeAttribute('aria-disabled'));
    }
  }

  buttons.forEach(button => button.addEventListener('click', () => act(button.id, false)));
  document.addEventListener('keydown', event => {
    const up = event.key === 'ArrowUp' || event.keyCode === 38;
    const down = event.key === 'ArrowDown' || event.keyCode === 40;
    if (up || down) {
      event.preventDefault();
      const current = buttons.indexOf(document.activeElement);
      const index = current < 0 ? 0 : (current + (up ? -1 : 1) + buttons.length) % buttons.length;
      buttons[index].focus();
    } else if (event.key === 'Enter' || event.keyCode === 13) {
      const selected = buttons.includes(document.activeElement) ? document.activeElement : buttons[0];
      event.preventDefault();
      if (!event.repeat) selected.click();
    }
  });

  buttons[0].focus();
  act('status', true);
  window.setInterval(() => {
    if (!document.hidden) act('status', true);
  }, 5000);
}());
