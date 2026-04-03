#[derive(Debug, Clone)]
pub(crate) struct VmMemoryBudget {
    pub(crate) max_stack: usize,
    pub(crate) max_globals: usize,
    pub(crate) max_ui_state_entries: usize,
    pub(crate) max_trace_events: usize,
    pub(crate) max_list_len: usize,
    pub(crate) max_map_len: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct VmMemoryStats {
    pub(crate) peak_stack: usize,
    pub(crate) peak_globals: usize,
    pub(crate) peak_ui_state_entries: usize,
    pub(crate) list_allocations: usize,
    pub(crate) map_allocations: usize,
    pub(crate) struct_allocations: usize,
    pub(crate) list_push_ops: usize,
    pub(crate) map_insert_ops: usize,
}

#[derive(Debug, Clone)]
pub struct VmMemorySnapshot {
    pub stack_len: usize,
    pub stack_peak: usize,
    pub globals_len: usize,
    pub globals_peak: usize,
    pub ui_state_len: usize,
    pub ui_state_peak: usize,
    pub trace_events_len: usize,
    pub list_allocations: usize,
    pub map_allocations: usize,
    pub struct_allocations: usize,
    pub list_push_ops: usize,
    pub map_insert_ops: usize,
    pub max_stack: usize,
    pub max_globals: usize,
    pub max_ui_state_entries: usize,
    pub max_trace_events: usize,
    pub max_list_len: usize,
    pub max_map_len: usize,
}

impl VmMemoryBudget {
    pub(crate) fn from_env() -> Self {
        Self {
            max_stack: read_budget("LUST_VM_MAX_STACK", 100_000),
            max_globals: read_budget("LUST_VM_MAX_GLOBALS", 20_000),
            max_ui_state_entries: read_budget("LUST_VM_MAX_UI_STATE", 20_000),
            max_trace_events: read_budget("LUST_VM_MAX_TRACE_EVENTS", 500),
            max_list_len: read_budget("LUST_VM_MAX_LIST_LEN", 200_000),
            max_map_len: read_budget("LUST_VM_MAX_MAP_LEN", 200_000),
        }
    }
}

fn read_budget(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}
