# Controlled synthetic detection-only corpus; never execute corpus files.
def sample( value=1 )

	return [ 'é  
text' ,value+1,{:key=>'v'}, __LINE__ ]

end
puts sample.inspect
puts __LINE__
value = <<~TEXT
  literal  


  #{[ 1 ,2 ].map{|entry|entry+1}}
TEXT
words = %W(one\ 


#{[ 1 ,2 ]})
pattern = /literal  


#{1+2}/
puts [value, words, pattern.source].inspect
__END__
raw  


