# coding: utf-8
# Copyright 2026 Test
def sample( value=1 )

	return [ 'é  
text' ,value+1,{:key=>'v'}, __LINE__ ]

end
puts sample.inspect
puts __LINE__
__END__
raw  

