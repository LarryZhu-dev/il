@id("model") module model {
  @id("model.Point") type Point = record { x: I32, y: I32 };
  @id("model.MaybePoint") type MaybePoint = Option<Point>;
  @id("model.Outcome") type Outcome = Result<Point, I32>;
  @id("model.Event") type Event = sum { Started, Moved(Point), Failed(I32) };
}
