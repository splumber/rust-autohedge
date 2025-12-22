use crate::agents::Agent;

pub struct ExecutionAgent;

impl Agent for ExecutionAgent {
    fn name(&self) -> &str {
        "Execution-Agent"
    }

    fn system_prompt(&self) -> &str {
        r#"You are an Execution Trader AI.
        
Format the final order based on the Risk Manager's output.
Output ONLY valid JSON. Do not include markdown formatting or chat text.

Output JSON:
{
    "action": "buy" | "sell",
    "symbol": "...",
    "qty": 10,
    "order_type": "market" | "limit",
    "limit_price": null
}
"#
    }
}
