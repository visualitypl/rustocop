# Examples adapted from RuboCop's bracket-spacing specs at 97f30c4.
# MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
array = [ a, [ b, c ] ]
array[0, 1] = [ 2 ]
empty = [ ]
array[]
array[[ 1, 2 ]]
case value
in ADT[ *head, ADT[ *rest, tail ] ]
end
literal = "[ x ] and array[ x ]"
words = %w[[ x ]]
text = <<~H
  [ x ] and array[ x ]
H
# rubocop:disable Layout/SpaceInsideArrayLiteralBrackets
kept = [ 3 ]
# rubocop:enable Layout/SpaceInsideArrayLiteralBrackets
# rustocop:disable Layout/SpaceInsideReferenceBrackets
array[ 3 ]
# rustocop:enable Layout/SpaceInsideReferenceBrackets
__END__
[ x ] and array[ x ]
