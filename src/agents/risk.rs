use crate::agents::Agent;

pub struct RiskAgent;

impl Agent for RiskAgent {
    fn name(&self) -> &str {
        "Risk-Manager"
    }

    fn system_prompt(&self) -> &str {
        r#"You are a Risk Manager AI.
        
Evaluate the trade proposal.
RULES:
1. Do NOT approve trades that use more than 5% of Buying Power/Cash.
2. Ensure Stop Loss is reasonable.

Output JSON:
{
    "approved": true | false,
    "position_size": 100,
    "stop_loss": 120.50,
    "take_profit": 140.00,
    "risk_reasoning": "..."
}
"#
    }
}
