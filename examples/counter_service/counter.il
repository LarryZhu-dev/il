entity counter {
    value: Int64
}

test "counter operations" {
    let c = counter { value: 0 }
    assert(c.value == 0)

    c.value = c.value + 1
    assert(c.value == 1)

    c.value = c.value - 1
    assert(c.value == 0)
}
