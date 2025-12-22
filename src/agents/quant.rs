use crate::agents::Agent;

pub struct QuantAgent;

impl Agent for QuantAgent {
    fn name(&self) -> &str {
        "Quant-Agent"
    }

    fn system_prompt(&self) -> &str {
        r#"You are a Quantitative Analyst AI. 
You will be provided with a Trading Thesis and Recent Market History.
Analyze the tabular data to calculate/estimate technical indicators.

Calculate and Output JSON:
{
    "technical_score": 0.0 to 1.0,
    "support_level": 123.45,
    "resistance_level": 130.00,
    "volatility_check": "pass" | "fail"
}
"#
    }
}
