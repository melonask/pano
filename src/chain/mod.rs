pub mod btc;
pub mod evm;
pub mod solana;

use crate::config::ChainConfig;
use crate::model::{ChainKind, DepositEvent, TargetMap};

/// Chain scanner trait — each blockchain implements this.
#[async_trait::async_trait]
pub trait ChainScanner {
    /// Return the CAIP-2 chain identifier.
    fn caip2(&self) -> &str;

    /// Return current blockchain tip height/slot.
    async fn get_tip(&self) -> anyhow::Result<u64>;

    /// Scan for deposits for targeted addresses only.
    /// Each entry in TargetMap carries fully resolved asset data
    /// (symbol, contract, decimals) for the scanner to use in RPC filters
    /// and asset matching.
    async fn scan(
        &self,
        from_block: u64,
        to_block: u64,
        targets: &TargetMap,
    ) -> anyhow::Result<Vec<DepositEvent>>;
}

/// Check if an address is allowed for a given asset symbol in the target map.
pub fn asset_allowed(targets: &TargetMap, address: &str, symbol: &str) -> bool {
    targets
        .get(address)
        .is_some_and(|assets| assets.iter().any(|a| a.symbol.eq_ignore_ascii_case(symbol)))
}

/// Create the appropriate scanner for a chain configuration.
pub fn create_scanner(config: &ChainConfig) -> anyhow::Result<Box<dyn ChainScanner + Send + Sync>> {
    let kind = ChainKind::from_caip2(&config.caip2)
        .ok_or_else(|| anyhow::anyhow!("unsupported chain: {}", config.caip2))?;
    match kind {
        ChainKind::Evm => Ok(Box::new(evm::EvmScanner::new(config.clone())?)),
        ChainKind::Solana => Ok(Box::new(solana::SolanaScanner::new(config.clone())?)),
        ChainKind::Bitcoin => Ok(Box::new(btc::BtcScanner::new(config.clone())?)),
    }
}
