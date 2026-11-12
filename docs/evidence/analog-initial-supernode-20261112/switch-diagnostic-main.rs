#![allow(dead_code)]
pub use labwired_core::{regulator,cosim};
mod analog {pub mod device;pub mod netlist;pub mod controlled_buck;pub mod opamp_power;pub mod initialization;mod initialization_charge;pub mod mna;pub use netlist::{AnalogError,Circuit,NodeRef,Waveform};}
use analog::{netlist::parse_netlist,mna::{Solver,Integration},initialization::AnalogInitialization};
fn main()->Result<(),Box<dyn std::error::Error>> {
 let path=std::env::args().nth(1).unwrap();let text=std::fs::read_to_string(path)?;let json:serde_json::Value=serde_json::from_str(&text)?;
 let cfg=&json["config"];let mut circuit=parse_netlist(cfg["netlist_text"].as_str().unwrap())?;
 let regulators=serde_json::from_value(cfg["regulators"].clone())?;analog::controlled_buck::bind_stages(&mut circuit,&regulators)?;
 let initial:AnalogInitialization=serde_json::from_value(cfg["initialization"].clone())?;
 match Solver::new_with_initialization(circuit,Integration::BackwardEuler,cfg["max_unknowns"].as_u64().unwrap() as usize,&initial) {Ok(s)=>println!("accepted {:?}",s.initialization_status()),Err(e)=>println!("refused {e}")};Ok(())
}
