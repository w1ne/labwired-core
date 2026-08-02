use labwired_core::network::Interconnect;
use labwired_core::peripherals::uart::UartStreamDevice;
use labwired_core::world::{MachineTrait, World};
use labwired_core::SimResult;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

struct OrderedMachine {
    id: String,
    steps: Arc<Mutex<Vec<String>>>,
}

impl MachineTrait for OrderedMachine {
    fn name(&self) -> &str {
        &self.id
    }

    fn step(&mut self) -> SimResult<()> {
        self.steps.lock().unwrap().push(self.id.clone());
        Ok(())
    }

    fn reset(&mut self) -> SimResult<()> {
        Ok(())
    }

    fn total_cycles(&self) -> u64 {
        0
    }

    fn read_u8(&self, _addr: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write_u8(&mut self, _addr: u64, _val: u8) -> SimResult<()> {
        Ok(())
    }

    fn attach_uart_stream(
        &mut self,
        _uart_id: &str,
        _dev: Box<dyn UartStreamDevice>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

struct CountingInterconnect(Arc<AtomicUsize>);

impl Interconnect for CountingInterconnect {
    fn tick(&mut self) -> SimResult<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn world_steps_nodes_in_lexical_order_then_ticks_each_interconnect_once() {
    // Repeat construction to rule out an accidentally lexical HashMap seed.
    for _ in 0..32 {
        let steps = Arc::new(Mutex::new(Vec::new()));
        let interconnect_ticks = Arc::new(AtomicUsize::new(0));
        let mut world = World::new("deterministic".to_string());

        for id in ["zeta", "alpha", "mu"] {
            world.add_machine(
                id.to_string(),
                Box::new(OrderedMachine {
                    id: id.to_string(),
                    steps: steps.clone(),
                }),
            );
        }
        world.add_interconnect(Box::new(CountingInterconnect(interconnect_ticks.clone())));

        let results = world.step_all();
        assert!(results.values().all(Result::is_ok));
        assert_eq!(
            *steps.lock().unwrap(),
            vec!["alpha", "mu", "zeta"],
            "node execution order must not depend on insertion or hash seeds"
        );
        assert_eq!(
            interconnect_ticks.load(Ordering::SeqCst),
            1,
            "the interconnect advances after one complete world round"
        );
    }
}
