use scry_sim::{Event, EventTracker};

pub struct StepTracker
{
	next_id: usize,
	events: Vec<Vec<Event>>,
}

impl StepTracker
{
	pub fn new() -> Self
	{
		StepTracker {
			next_id: 0,
			events: vec![vec![]],
		}
	}

	pub fn all_events(&self) -> impl Iterator<Item = impl Iterator<Item = &Event> + '_>
	{
		self.events.iter().map(|events| events.iter())
	}

	/// Deletes all events.
	pub fn clear_events(&mut self)
	{
		self.events.clear();
		self.step();
	}
}

impl EventTracker for StepTracker
{
	fn new_id(&mut self) -> usize
	{
		let id = self.next_id;
		self.next_id += 1;
		id
	}

	fn event(&mut self, event: Event)
	{
		self.events.last_mut().unwrap().push(event);
	}

	fn step(&mut self)
	{
		self.events.push(vec![]);
	}
}
