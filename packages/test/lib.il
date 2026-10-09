@id("test") module test visibility public imports [core] {
 @id("test.TestError") type TestError=sum {Assertion};
 @id("test.Result") type Result=Result<Unit,TestError>;
 @id("test.assert_true") fn assert_true(condition:Bool)->Result {if condition{return Ok(());}return Err(TestError::Assertion());}
 @id("test.assert_i64") fn assert_i64(actual:I64,expected:I64)->Result {return assert_true(actual==expected);}
 @id("test.assert_bytes") fn assert_bytes(actual:Bytes,expected:Bytes)->Result {return assert_true(runtime.bytes_equal(actual,expected));}
}
