//! SYNC `Await` / `AwaitFence`: client suspension and the counter / fence
//! trigger plumbing that ends it (Xorg `Xext/sync.c`).
//!
//! A suspended client has an entry in `ServerState::sync_awaits`. The core
//! loop's fair request queue treats such a client as not runnable — its
//! later requests stay queued, in order, behind the await (Xorg
//! `IgnoreClient`) while every other client keeps running. The await ends
//! when one of its triggers fires: a counter changes (`SetCounter`,
//! `ChangeCounter`, the SERVERTIME / IDLETIME system counters advancing),
//! a fence triggers (`TriggerFence`, a Present idle fence), or a counter or
//! fence it names is destroyed. Firing sends the `CounterNotify` events
//! Xorg's `SyncAwaitTriggerFired` would and removes the entry, which makes
//! the client runnable again (`AttendClient`). Nothing here ever blocks the
//! loop itself.

use yserver_protocol::x11::{ClientId, sync as x11sync};

use crate::{
    core_loop::fanout::fanout_event_to_clients,
    server::{ServerState, SyncAwait, SyncAwaitCondition},
};

/// SERVERTIME and the IDLETIME family: server-owned counters whose value
/// comes from the clock, which clients may read and wait on but never set
/// or destroy.
#[must_use]
pub(crate) fn is_system_counter(counter: u32) -> bool {
    counter == x11sync::SERVERTIME_COUNTER || is_idletime_counter(counter)
}

fn is_idletime_counter(counter: u32) -> bool {
    matches!(
        counter,
        x11sync::IDLETIME_COUNTER | x11sync::IDLETIME_DEVICE_VCP | x11sync::IDLETIME_DEVICE_VCK
    )
}

/// Current value of counter `counter`, system or client-created. `None`
/// when no such counter exists (the caller's BadCounter).
#[must_use]
pub(crate) fn counter_value(state: &ServerState, counter: u32) -> Option<i64> {
    if counter == x11sync::SERVERTIME_COUNTER {
        return Some(i64::from(state.timestamp_now()));
    }
    if is_idletime_counter(counter) {
        return Some(crate::core_loop::process_request::idletime_current_idle(
            state, counter,
        ));
    }
    state.sync_counters.get(&counter).map(|c| c.value)
}

/// Whether `client`'s request stream is suspended by an await.
#[must_use]
pub(crate) fn client_is_suspended(state: &ServerState, client: ClientId) -> bool {
    state.sync_awaits.contains_key(&client.0)
}

/// Xorg `SyncAwaitEpilogue`: suspend `client` on `conditions`, then test
/// each trigger against the current state (a counter's old value is its
/// current value, so only comparisons can already hold); the first one
/// that holds fires the await straight away.
pub(crate) fn begin_await(
    state: &mut ServerState,
    client: ClientId,
    conditions: Vec<SyncAwaitCondition>,
) {
    // Xorg's `SyncInitTrigger` re-reads a system counter into its value
    // slot, so later changes are measured from here. Mirror that for the
    // clock-driven counters' last-evaluated values.
    for condition in &conditions {
        if let SyncAwaitCondition::Counter { counter, .. } = *condition {
            if counter == x11sync::SERVERTIME_COUNTER {
                state.sync_servertime_last = Some(i64::from(state.timestamp_now()));
            } else if is_idletime_counter(counter) {
                let idle = crate::core_loop::process_request::idletime_current_idle(state, counter);
                state.idletime_last_evaluated.insert(counter, idle);
            }
        }
    }
    let already = conditions.iter().any(|condition| match *condition {
        SyncAwaitCondition::Counter {
            counter,
            test_type,
            test_value,
            ..
        } => counter_value(state, counter)
            .is_some_and(|value| x11sync::trigger_fires(test_type, value, value, test_value)),
        SyncAwaitCondition::Fence { fence } => {
            state.sync_fences.get(&fence).is_some_and(|f| f.triggered)
        }
    });
    state.sync_awaits.insert(client.0, SyncAwait { conditions });
    if already {
        fire_await(state, client, None);
    }
}

/// Xorg `SyncAwaitTriggerFired`: send `client` the `CounterNotify` events
/// for its await, contiguously and with a descending `count`, then resume
/// it. `destroyed` names the counter or fence being destroyed (with a
/// counter's last value), if that is what fired the await.
///
/// An event is sent for every condition on the object being destroyed
/// (`destroyed` set), and for every counter condition whose difference
/// `value - test_value` reaches the event threshold: at least it for the
/// positive test types, at most it for the negative ones. A difference that
/// overflows an INT64 sends nothing. Fences only report destruction.
pub(crate) fn fire_await(state: &mut ServerState, client: ClientId, destroyed: Option<(u32, i64)>) {
    let Some(await_) = state.sync_awaits.remove(&client.0) else {
        return;
    };
    // (object, wait value, counter value, destroyed)
    let mut events: Vec<(u32, i64, i64, bool)> = Vec::new();
    for condition in &await_.conditions {
        let object = condition.object();
        if let Some((gone, last)) = destroyed
            && gone == object
        {
            let (wait, value) = match *condition {
                SyncAwaitCondition::Counter { test_value, .. } => (test_value, last),
                SyncAwaitCondition::Fence { .. } => (0, 0),
            };
            events.push((object, wait, value, true));
            continue;
        }
        let SyncAwaitCondition::Counter {
            counter,
            test_type,
            test_value,
            event_threshold,
        } = *condition
        else {
            continue;
        };
        let Some(value) = counter_value(state, counter) else {
            continue;
        };
        let Some(diff) = value.checked_sub(test_value) else {
            continue;
        };
        let report = match test_type {
            x11sync::TEST_POSITIVE_COMPARISON | x11sync::TEST_POSITIVE_TRANSITION => {
                diff >= event_threshold
            }
            _ => diff <= event_threshold,
        };
        if report {
            events.push((counter, test_value, value, false));
        }
    }
    log::debug!(
        "sync: await of client {} fired ({} CounterNotify), request stream resumes",
        client.0,
        events.len(),
    );
    if events.is_empty() {
        return;
    }
    let time = state.timestamp_now();
    let total = events.len();
    let _dropped = fanout_event_to_clients(state, &[client], |buf, seq, order| {
        for (index, (object, wait, value, gone)) in events.iter().enumerate() {
            let remaining = u16::try_from(total - index - 1).unwrap_or(u16::MAX);
            buf.extend_from_slice(&x11sync::encode_counter_notify_event(
                order,
                crate::nested::SYNC_FIRST_EVENT,
                seq,
                *object,
                *wait,
                *value,
                time,
                remaining,
                *gone,
            ));
        }
    });
}

/// Clients whose await has a condition matching `pred`, in client order.
fn awaiting_clients(state: &ServerState, pred: impl Fn(&SyncAwaitCondition) -> bool) -> Vec<u32> {
    let mut clients: Vec<u32> = state
        .sync_awaits
        .iter()
        .filter(|(_, a)| a.conditions.iter().any(&pred))
        .map(|(client, _)| *client)
        .collect();
    clients.sort_unstable();
    clients
}

/// Xorg `SyncChangeCounter`: counter `counter` went from `old` to `new`.
/// Runs its alarms, then fires every await with a trigger on it that now
/// holds.
pub(crate) fn counter_changed(state: &mut ServerState, counter: u32, old: i64, new: i64) {
    crate::core_loop::process_request::evaluate_alarms_for_counter(state, counter, old, new);
    let fired = awaiting_clients(state, |condition| match *condition {
        SyncAwaitCondition::Counter {
            counter: c,
            test_type,
            test_value,
            ..
        } => c == counter && x11sync::trigger_fires(test_type, old, new, test_value),
        SyncAwaitCondition::Fence { .. } => false,
    });
    for client in fired {
        fire_await(state, ClientId(client), None);
    }
}

/// Xorg `FreeCounter`: `counter` (last value `last`) is gone. Its alarms
/// go Inactive with an AlarmNotify and stop watching it
/// (`SyncAlarmCounterDestroyed`); every await naming it fires with a
/// destroyed `CounterNotify`.
pub(crate) fn counter_destroyed(state: &mut ServerState, counter: u32, last: i64) {
    let mut alarms: Vec<u32> = state
        .sync_alarms
        .iter()
        .filter(|(_, a)| a.counter == counter)
        .map(|(id, _)| *id)
        .collect();
    alarms.sort_unstable();
    let time = state.timestamp_now();
    for alarm_id in alarms {
        let Some(alarm) = state.sync_alarms.get_mut(&alarm_id) else {
            continue;
        };
        alarm.state = x11sync::ALARM_STATE_INACTIVE;
        alarm.counter = 0;
        let (owner, events, wait) = (alarm.owner, alarm.events, alarm.wait_value);
        if events {
            let _dropped = fanout_event_to_clients(state, &[owner], |buf, seq, order| {
                buf.extend_from_slice(&x11sync::encode_alarm_notify_event(
                    order,
                    crate::nested::SYNC_FIRST_EVENT,
                    seq,
                    alarm_id,
                    last,
                    wait,
                    time,
                    x11sync::ALARM_STATE_INACTIVE,
                ));
            });
        }
    }
    let fired = awaiting_clients(
        state,
        |condition| matches!(*condition, SyncAwaitCondition::Counter { counter: c, .. } if c == counter),
    );
    for client in fired {
        fire_await(state, ClientId(client), Some((counter, last)));
    }
}

/// Xorg `miSyncTriggerFence`: mark `fence` triggered and fire every await
/// waiting on it. Unknown fences are ignored (a Present idle fence the
/// client already destroyed).
pub(crate) fn fence_triggered(state: &mut ServerState, fence: u32) {
    let Some(f) = state.sync_fences.get_mut(&fence) else {
        return;
    };
    f.triggered = true;
    let fired = awaiting_clients(
        state,
        |condition| matches!(*condition, SyncAwaitCondition::Fence { fence: f } if f == fence),
    );
    for client in fired {
        fire_await(state, ClientId(client), None);
    }
}

/// Xorg `miSyncDestroyFence`: every await naming `fence` fires with a
/// destroyed `CounterNotify` (counter = the fence, values 0). The caller
/// removes the fence itself.
pub(crate) fn fence_destroyed(state: &mut ServerState, fence: u32) {
    let fired = awaiting_clients(
        state,
        |condition| matches!(*condition, SyncAwaitCondition::Fence { fence: f } if f == fence),
    );
    for client in fired {
        fire_await(state, ClientId(client), Some((fence, 0)));
    }
}

/// Whether anything watches SERVERTIME: an active alarm or an await.
fn servertime_watched(state: &ServerState) -> bool {
    state
        .sync_alarms
        .values()
        .any(|a| a.counter == x11sync::SERVERTIME_COUNTER && a.state == x11sync::ALARM_STATE_ACTIVE)
        || counter_awaited(state, x11sync::SERVERTIME_COUNTER)
}

/// Whether some await has a condition on counter `counter`.
fn counter_awaited(state: &ServerState, counter: u32) -> bool {
    state.sync_awaits.values().any(|a| {
        a.conditions.iter().any(
            |condition| matches!(*condition, SyncAwaitCondition::Counter { counter: c, .. } if c == counter),
        )
    })
}

/// Post-poll SERVERTIME pass (Xorg `ServertimeWakeupHandler` →
/// `SyncChangeCounter`): feed `(last evaluated, now)` through the alarm
/// and await triggers watching SERVERTIME.
pub(crate) fn evaluate_servertime(state: &mut ServerState) {
    if !servertime_watched(state) {
        state.sync_servertime_last = None;
        return;
    }
    let now = i64::from(state.timestamp_now());
    let old = state.sync_servertime_last.unwrap_or(now);
    state.sync_servertime_last = Some(now);
    if old != now {
        counter_changed(state, x11sync::SERVERTIME_COUNTER, old, now);
    }
}

/// Earliest instant a clock-driven trigger can fire: SERVERTIME alarms and
/// awaits reaching their value, and IDLETIME awaits (positive tests)
/// reaching theirs. (IDLETIME alarms have their own deadline; negative
/// IDLETIME tests fire on input.) Joins the core loop's poll timeout the
/// way Xorg's block handlers shorten the select timeout.
#[must_use]
pub(crate) fn system_counter_deadline(state: &ServerState) -> Option<std::time::Instant> {
    let positive = |test_type: u32| {
        matches!(
            test_type,
            x11sync::TEST_POSITIVE_TRANSITION | x11sync::TEST_POSITIVE_COMPARISON
        )
    };
    // A trigger is still pending while its value lies beyond the value the
    // last post-poll pass evaluated; its deadline may already be due (the
    // clock passed it since), which the caller turns into a zero timeout.
    // Values at or below the last evaluated one either fired already or
    // can never fire again (a transition that was already crossed), so they
    // must not keep the loop awake.
    let now_instant = std::time::Instant::now();
    let now_ms = i64::from(state.timestamp_now());
    let servertime_last = state.sync_servertime_last.unwrap_or(now_ms);
    let servertime_at = |test_value: i64| {
        if test_value <= servertime_last {
            return None;
        }
        let ahead = u64::try_from(test_value.saturating_sub(now_ms)).unwrap_or(0);
        Some(now_instant + std::time::Duration::from_millis(ahead))
    };
    let mut deadlines: Vec<std::time::Instant> = state
        .sync_alarms
        .values()
        .filter(|a| {
            a.counter == x11sync::SERVERTIME_COUNTER
                && a.state == x11sync::ALARM_STATE_ACTIVE
                && positive(u32::from(a.test_type))
        })
        .filter_map(|a| servertime_at(a.wait_value))
        .collect();
    let idle_suspended = !state.screensaver.suspend_counts.is_empty();
    for await_ in state.sync_awaits.values() {
        for condition in &await_.conditions {
            let SyncAwaitCondition::Counter {
                counter,
                test_type,
                test_value,
                ..
            } = *condition
            else {
                continue;
            };
            if !positive(test_type) {
                continue;
            }
            if counter == x11sync::SERVERTIME_COUNTER {
                deadlines.extend(servertime_at(test_value));
            } else if is_idletime_counter(counter) && !idle_suspended {
                let last = state
                    .idletime_last_evaluated
                    .get(&counter)
                    .copied()
                    .unwrap_or(0);
                if test_value > last
                    && let Ok(ms) = u64::try_from(test_value)
                {
                    deadlines.push(
                        state.idletime_baseline(counter) + std::time::Duration::from_millis(ms),
                    );
                }
            }
        }
    }
    deadlines.into_iter().min()
}

/// Whether an await watches IDLETIME counter `counter` (the post-poll
/// IDLETIME pass must run for it even without alarms).
#[must_use]
pub(crate) fn idletime_awaited(state: &ServerState, counter: u32) -> bool {
    counter_awaited(state, counter)
}
