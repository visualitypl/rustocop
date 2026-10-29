raise ArgumentError, "fail" # comment
Kernel.raise
::Kernel.raise("bad")
obj.fail
fail # rubocop:disable Style/SignalException
