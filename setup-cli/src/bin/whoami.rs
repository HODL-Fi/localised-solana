// Prints what this build actually selected. Guards against a mainnet run silently using
// devnet ids — the failure mode that makes a program look alive and be unusable.
fn main() {
    println!("  program id     {}", hodl_loans::ID);
    println!("  switchboard    {}", hodl_loans::SWITCHBOARD_ON_DEMAND_PID);
    println!("  devnet feature {}", cfg!(feature = "devnet"));
}
