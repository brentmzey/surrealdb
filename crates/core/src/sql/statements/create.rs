use crate::ctx::Context;
use crate::dbs::{Iterator, Options, Statement};
use crate::doc::CursorDoc;
use crate::err::Error;
use crate::idx::planner::{QueryPlanner, RecordStrategy, StatementContext};
use crate::sql::{Data, FlowResultExt as _, Output, Timeout, Value, Values, Version};

use reblessive::tree::Stk;
use revision::revisioned;
use serde::{Deserialize, Serialize};
use std::fmt;

#[revisioned(revision = 3)]
#[derive(Clone, Debug, Default, Eq, PartialEq, PartialOrd, Serialize, Deserialize, Hash)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[non_exhaustive]
pub struct CreateStatement {
	// A keyword modifier indicating if we are expecting a single result or several
	#[revision(start = 2)]
	pub only: bool,
	// Where we are creating (i.e. table, or record ID)
	pub what: Values,
	// The data associated with the record being created
	pub data: Option<Data>,
	//  What the result of the statement should resemble (i.e. Diff or no result etc).
	pub output: Option<Output>,
	// The timeout for the statement
	pub timeout: Option<Timeout>,
	// If the statement should be run in parallel
	pub parallel: bool,
	// Version as nanosecond timestamp passed down to Datastore
	#[revision(start = 3)]
	pub version: Option<Version>,
}

impl CreateStatement {
	/// Check if we require a writeable transaction
	pub(crate) fn writeable(&self) -> bool {
		true
	}
	/// Process this type returning a computed simple Value
	pub(crate) async fn compute(
		&self,
		stk: &mut Stk,
		ctx: &Context,
		opt: &Options,
		doc: Option<&CursorDoc>,
	) -> Result<Value, Error> {
		// Valid options?
		opt.valid_for_db()?;
		// Create a new iterator
		let mut i = Iterator::new();
		// Assign the statement
		let stm = Statement::from(self);
		// Propagate the version to the underlying datastore
		let version = match &self.version {
			Some(v) => Some(v.compute(stk, ctx, opt, doc).await?),
			_ => None,
		};
		// Ensure futures are stored
		let opt = &opt.new_with_futures(false).with_version(version);
		// Check if there is a timeout
		let ctx = stm.setup_timeout(ctx)?;
		// Get a query planner
		let mut planner = QueryPlanner::new();
		let stm_ctx = StatementContext::new(&ctx, opt, &stm)?;
		// Loop over the create targets
		for w in self.what.0.iter() {
			let v = w.compute(stk, &ctx, opt, doc).await.catch_return()?;
			i.prepare(stk, &mut planner, &stm_ctx, v).await.map_err(|e| match e {
				Error::InvalidStatementTarget {
					value: v,
				} => Error::CreateStatement {
					value: v,
				},
				e => e,
			})?;
		}
		// Attach the query planner to the context
		let ctx = stm.setup_query_planner(planner, ctx);
		// Process the statement
		let res = i.output(stk, &ctx, opt, &stm, RecordStrategy::KeysAndValues).await?;
		// Catch statement timeout
		if ctx.is_timedout()? {
			return Err(Error::QueryTimedout);
		}
		// Output the results
		match res {
			// This is a single record result
			Value::Array(mut a) if self.only => match a.len() {
				// There was exactly one result
				1 => Ok(a.remove(0)),
				// There were no results
				_ => Err(Error::SingleOnlyOutput),
			},
			// This is standard query result
			v => Ok(v),
		}
	}
}

impl fmt::Display for CreateStatement {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "CREATE")?;
		if self.only {
			f.write_str(" ONLY")?
		}
		write!(f, " {}", self.what)?;
		if let Some(ref v) = self.data {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.output {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.version {
			write!(f, " {v}")?
		}
		if let Some(ref v) = self.timeout {
			write!(f, " {v}")?
		}
		if self.parallel {
			f.write_str(" PARALLEL")?
		}
		Ok(())
	}
}
