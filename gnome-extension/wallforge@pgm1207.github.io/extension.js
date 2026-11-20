// WallForge for GNOME Shell.
//
// GNOME Shell has no wlr-layer-shell, so an external program cannot put a surface behind the windows. WallForge's
// renderer runs instead as an ordinary window titled "@wallforge!<connector>" (wallforge-render --window). This
// extension keeps that window minimized and out of every window list, and draws a live Clutter.Clone of it inside
// GNOME's own background actor for the monitor named by <connector>: on the desktop, in the overview and in the
// workspace thumbnails. Without a renderer window for a monitor, GNOME's normal background shows.
//
// The approach (minimized renderer + clone in the background actor) is the one gnome-ext-hanabi proved out.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Background from 'resource:///org/gnome/shell/ui/background.js';
import * as Workspace from 'resource:///org/gnome/shell/ui/workspace.js';
import * as WorkspaceThumbnail from 'resource:///org/gnome/shell/ui/workspaceThumbnail.js';
import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';

const TAG = '@wallforge!';

function isRenderer(win) {
    const title = win?.get_title?.();
    return typeof title === 'string' && title.startsWith(TAG);
}

function connectorOf(win) {
    return win.get_title().slice(TAG.length).split('|')[0];
}

// One per GNOME background actor (desktop, overview workspace, thumbnail): holds the clone for its monitor.
class Slot {
    constructor(owner, backgroundActor) {
        this._owner = owner;
        this._bg = backgroundActor;
        this._clone = null;
        this._destroyId = 0;
        this._box = new Clutter.Actor({
            layout_manager: new Clutter.BinLayout(),
            x_expand: true,
            y_expand: true,
        });
        backgroundActor.layout_manager = new Clutter.BinLayout();
        backgroundActor.add_child(this._box);
        this._destroyId = this._box.connect('destroy', () => {
            this._destroyId = 0;
            this._clone = null;
            this._owner.slots.delete(this);
        });
        this.refresh();
    }

    refresh() {
        const source = this._owner.rendererFor(this._bg.monitor);
        if (!source) {
            this._clone?.destroy();
            this._clone = null;
            return;
        }
        if (this._clone?.source === source)
            return;
        if (this._clone) {
            this._clone.set_source(source);
        } else {
            this._clone = new Clutter.Clone({source, x_expand: true, y_expand: true});
            this._box.add_child(this._clone);
        }
    }

    destroy() {
        if (this._destroyId) {
            this._box.disconnect(this._destroyId);
            this._destroyId = 0;
        }
        this._clone?.destroy();
        this._clone = null;
        this._box.destroy();
        this._owner.slots.delete(this);
    }
}

export default class WallForgeExtension extends Extension {
    enable() {
        this.slots = new Set();
        this._injections = new InjectionManager();
        this._signals = [];
        this._refreshId = 0;
        this._hideIds = new Set();
        this._watched = new Map();   // Meta.Window -> {ids, unmanagedId}
        this._actorIds = new Map();  // Meta.WindowActor -> 'destroy' handler id
        this._stuck = new Set();     // windows this extension made sticky
        this._allWindowActors = () => global.get_window_actors();

        this._installOverrides();

        this._signals.push([global.window_manager,
            global.window_manager.connect_after('map', (_wm, actor) => this._onMap(actor))]);
        this._signals.push([global.display,
            global.display.connect('window-created', (_d, win) => this._watch(win))]);
        for (const actor of this._allWindowActors())
            this._watch(actor.get_meta_window());

        // Recreate the existing backgrounds so the injected _createBackgroundActor runs for them.
        Main.layoutManager._updateBackgrounds();
    }

    disable() {
        if (this._refreshId)
            GLib.source_remove(this._refreshId);
        this._refreshId = 0;
        for (const id of this._hideIds)
            GLib.source_remove(id);
        this._hideIds.clear();
        for (const [obj, id] of this._signals)
            obj.disconnect(id);
        this._signals = [];
        for (const win of [...this._watched.keys()])
            this._unwatch(win);
        for (const [actor, id] of this._actorIds)
            actor.disconnect(id);
        this._actorIds.clear();
        for (const win of this._stuck) {
            if (win.get_compositor_private())
                win.unstick();
        }
        this._stuck.clear();
        for (const slot of [...this.slots])
            slot.destroy();
        this.slots.clear();
        this._injections.clear();
        this._injections = null;
        this._allWindowActors = null;
        this._watched = null;
        this._actorIds = null;
        this._hideIds = null;
        this._stuck = null;
        this.slots = null;
        Main.layoutManager._updateBackgrounds();
    }

    // The newest renderer window for a monitor: during a switch the old and new renderer briefly coexist, and the
    // clone should follow the new one as soon as it has a frame (a Wayland window maps only once it has one).
    rendererFor(monitorIndex) {
        const manager = global.backend.get_monitor_manager();
        let best = null;
        for (const actor of this._allWindowActors()) {
            const win = actor.get_meta_window();
            if (!isRenderer(win) || actor.is_destroyed?.())
                continue;
            if (manager.get_monitor_for_connector(connectorOf(win)) !== monitorIndex)
                continue;
            if (!best || win.get_stable_sequence() > best.get_meta_window().get_stable_sequence())
                best = actor;
        }
        return best;
    }

    _scheduleRefresh() {
        if (this._refreshId)
            return;
        this._refreshId = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            this._refreshId = 0;
            for (const slot of this.slots)
                slot.refresh();
            return GLib.SOURCE_REMOVE;
        });
    }

    _onMap(actor) {
        const win = actor.get_meta_window();
        if (!isRenderer(win))
            return;
        if (!this._actorIds.has(actor)) {
            this._actorIds.set(actor, actor.connect('destroy', () => {
                this._actorIds?.delete(actor);
                this._scheduleRefresh();
            }));
        }
        // Let the first frame reach the actor's texture before minimizing: a window minimized before it was ever
        // shown never gets one for the clone.
        this._scheduleHide(win);
        this._scheduleRefresh();
    }

    // Keep a renderer window out of sight: on every workspace, minimized, never above anything.
    _watch(win) {
        if (!win || this._watched.has(win))
            return;
        const ids = [
            win.connect('notify::title', () => {
                if (isRenderer(win) && win.get_compositor_private())
                    this._scheduleHide(win);
            }),
            win.connect('notify::minimized', () => {
                if (isRenderer(win) && !win.minimized)
                    win.minimize();
            }),
            win.connect('notify::above', () => {
                if (isRenderer(win) && win.above)
                    win.unmake_above();
            }),
        ];
        const unmanagedId = win.connect('unmanaged', () => this._unwatch(win));
        this._watched.set(win, {ids, unmanagedId});
    }

    _unwatch(win) {
        const entry = this._watched?.get(win);
        if (!entry)
            return;
        this._watched.delete(win);
        for (const id of [...entry.ids, entry.unmanagedId])
            win.disconnect(id);
        this._stuck?.delete(win);
    }

    _scheduleHide(win) {
        const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
            this._hideIds.delete(id);
            if (win.get_compositor_private()) {
                // Mutter sizes a new window to the work area (minus the top bar); the wallpaper covers the monitor.
                const m = win.get_monitor() >= 0 ? global.display.get_monitor_geometry(win.get_monitor()) : null;
                if (m)
                    win.move_resize_frame(false, m.x, m.y, m.width, m.height);
                this._hide(win);
            }
            this._scheduleRefresh();
            return GLib.SOURCE_REMOVE;
        });
        this._hideIds.add(id);
    }

    _hide(win) {
        if (!win.is_on_all_workspaces()) {
            win.stick();
            this._stuck.add(win);
        }
        if (win.above)
            win.unmake_above();
        if (!win.minimized)
            win.minimize();
    }

    _installOverrides() {
        const self = this;
        const inj = this._injections;

        // A clone of the right renderer inside every background actor GNOME creates.
        inj.overrideMethod(Background.BackgroundManager.prototype, '_createBackgroundActor', original => {
            return function (...args) {
                const backgroundActor = original.apply(this, args);
                self.slots.add(new Slot(self, backgroundActor));
                return backgroundActor;
            };
        });

        // The renderer windows are an implementation detail: hide them from every list GNOME shows the user.
        inj.overrideMethod(Shell.Global.prototype, 'get_window_actors', original => {
            self._allWindowActors = () => original.call(global);
            return function (...args) {
                return original.apply(this, args).filter(a => !isRenderer(a.get_meta_window()));
            };
        });
        for (const proto of [Workspace.Workspace.prototype, WorkspaceThumbnail.WorkspaceThumbnail.prototype]) {
            inj.overrideMethod(proto, '_isOverviewWindow', original => {
                return function (win, ...rest) {
                    return isRenderer(win) ? false : original.call(this, win, ...rest);
                };
            });
        }
        inj.overrideMethod(Meta.Display.prototype, 'get_tab_list', original => {
            return function (...args) {
                return original.apply(this, args).filter(w => !isRenderer(w));
            };
        });
        inj.overrideMethod(Shell.WindowTracker.prototype, 'get_window_app', original => {
            return function (win, ...rest) {
                return isRenderer(win) ? null : original.call(this, win, ...rest);
            };
        });
        inj.overrideMethod(Shell.App.prototype, 'get_windows', original => {
            return function (...args) {
                return original.apply(this, args).filter(w => !isRenderer(w));
            };
        });
    }
}
